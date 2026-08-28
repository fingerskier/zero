//! v0.1 query evaluator per doc/SCHEMA.md §5.
//!
//! Evaluates a parsed [`crate::query::Query`] over a fixture graph with the
//! deterministic read semantics the spec pins: two-valued null logic in WHERE,
//! cross-type comparisons false (except int/float numeric), one total order for
//! ORDER BY, and MVRegister conflict surfacing. Every result order is identical
//! on every peer (I-1 extended to reads).

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};

use crate::query::{Cmp, Dir, Expr, Item, Lit, Operand, Query};

/// A query-visible value. `Mv` is an MVRegister's concurrent set; `Node`/`Edge`
/// are entity references produced by projecting a bare variable.
#[derive(Debug, Clone, PartialEq)]
pub enum QValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    Bytes(Vec<u8>),
    Mv(Vec<QValue>),
    Node(String),
    Edge(String),
}

pub struct GNode {
    pub id: String,
    pub label: String,
    pub props: BTreeMap<String, QValue>,
}

pub struct GEdge {
    pub id: String,
    pub label: String,
    pub src: String,
    pub dst: String,
    pub props: BTreeMap<String, QValue>,
}

#[derive(Default)]
pub struct Graph {
    pub nodes: Vec<GNode>,
    pub edges: Vec<GEdge>,
    node_by_id: HashMap<String, usize>,
    edge_by_id: HashMap<String, usize>,
}

#[derive(Clone)]
enum Ref {
    Node(String),
    Edge(String),
}

impl Graph {
    pub fn new(nodes: Vec<GNode>, edges: Vec<GEdge>) -> Self {
        let node_by_id = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        let edge_by_id = edges
            .iter()
            .enumerate()
            .map(|(i, e)| (e.id.clone(), i))
            .collect();
        Self {
            nodes,
            edges,
            node_by_id,
            edge_by_id,
        }
    }

    fn node(&self, id: &str) -> Option<&GNode> {
        self.node_by_id.get(id).and_then(|&i| self.nodes.get(i))
    }
    fn edge(&self, id: &str) -> Option<&GEdge> {
        self.edge_by_id.get(id).and_then(|&i| self.edges.get(i))
    }
}

/// From `Lit` (parser literal) to a runtime value.
fn lit_value(l: &Lit) -> QValue {
    match l {
        Lit::Null => QValue::Null,
        Lit::Bool(b) => QValue::Bool(*b),
        Lit::Int(i) => QValue::Int(*i),
        Lit::Float(f) => QValue::Float(*f),
        Lit::Text(s) => QValue::Text(s.clone()),
    }
}

fn as_f64(v: &QValue) -> Option<f64> {
    match v {
        QValue::Int(i) => Some(*i as f64),
        QValue::Float(f) => Some(*f),
        _ => None,
    }
}

/// Collapse an MVRegister for a WHERE comparison: a singleton unwraps to its
/// element; any other size makes the comparison false (SCHEMA.md §5).
fn collapse_for_cmp(v: &QValue) -> Option<QValue> {
    match v {
        QValue::Mv(xs) if xs.len() == 1 => Some(xs[0].clone()),
        QValue::Mv(_) => None,
        other => Some(other.clone()),
    }
}

/// WHERE comparison — two-valued: null or cross-type (except int/float numeric)
/// yields false; an MVRegister with ≠1 value yields false.
fn cmp_predicate(a: &QValue, op: Cmp, b: &QValue) -> bool {
    let (Some(a), Some(b)) = (collapse_for_cmp(a), collapse_for_cmp(b)) else {
        return false;
    };
    if matches!(a, QValue::Null) || matches!(b, QValue::Null) {
        return false;
    }
    let ord = match (&a, &b) {
        _ if as_f64(&a).is_some() && as_f64(&b).is_some() => {
            as_f64(&a).unwrap().partial_cmp(&as_f64(&b).unwrap())
        }
        (QValue::Bool(x), QValue::Bool(y)) => Some(x.cmp(y)),
        (QValue::Text(x), QValue::Text(y)) => Some(x.as_bytes().cmp(y.as_bytes())),
        (QValue::Bytes(x), QValue::Bytes(y)) => Some(x.cmp(y)),
        _ => return false, // cross-type
    };
    let Some(ord) = ord else {
        return false; // NaN
    };
    match op {
        Cmp::Eq => ord == Ordering::Equal,
        Cmp::Ne => ord != Ordering::Equal,
        Cmp::Lt => ord == Ordering::Less,
        Cmp::Le => ord != Ordering::Greater,
        Cmp::Gt => ord == Ordering::Greater,
        Cmp::Ge => ord != Ordering::Less,
    }
}

fn type_rank(v: &QValue) -> u8 {
    match v {
        QValue::Null => 0,
        QValue::Bool(_) => 1,
        QValue::Int(_) | QValue::Float(_) => 2,
        QValue::Text(_) => 3,
        QValue::Bytes(_) => 4,
        QValue::Mv(_) => 5,
        QValue::Node(_) => 6,
        QValue::Edge(_) => 7,
    }
}

/// The single ORDER BY total order (SCHEMA.md §5): null < bool < int/float
/// (numeric) < text (bytewise UTF-8) < bytes (bytewise). Total across types.
fn total_order(a: &QValue, b: &QValue) -> Ordering {
    let (ra, rb) = (type_rank(a), type_rank(b));
    if ra != rb {
        return ra.cmp(&rb);
    }
    match (a, b) {
        (QValue::Null, QValue::Null) => Ordering::Equal,
        (QValue::Bool(x), QValue::Bool(y)) => x.cmp(y),
        _ if as_f64(a).is_some() && as_f64(b).is_some() => as_f64(a)
            .unwrap()
            .partial_cmp(&as_f64(b).unwrap())
            .unwrap_or(Ordering::Equal),
        (QValue::Text(x), QValue::Text(y)) => x.as_bytes().cmp(y.as_bytes()),
        (QValue::Bytes(x), QValue::Bytes(y)) => x.cmp(y),
        (QValue::Mv(x), QValue::Mv(y)) => {
            for (xe, ye) in x.iter().zip(y.iter()) {
                let o = total_order(xe, ye);
                if o != Ordering::Equal {
                    return o;
                }
            }
            x.len().cmp(&y.len())
        }
        (QValue::Node(x), QValue::Node(y)) => x.cmp(y),
        (QValue::Edge(x), QValue::Edge(y)) => x.cmp(y),
        _ => Ordering::Equal,
    }
}

type Binding = BTreeMap<String, Ref>;

fn label_ok(want: &Option<String>, have: &str) -> bool {
    want.as_ref().is_none_or(|w| w == have)
}

/// Bind the MATCH pattern deterministically (nodes/edges scanned in id order).
fn bind(query: &Query, graph: &Graph) -> Vec<Binding> {
    let pat = &query.pattern;
    let mut nodes: Vec<&GNode> = graph.nodes.iter().collect();
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    let mut out = Vec::new();

    let Some((edge_pat, right)) = &pat.hop else {
        for n in nodes {
            if label_ok(&pat.left.label, &n.label) {
                let mut b = Binding::new();
                b.insert(pat.left.var.clone(), Ref::Node(n.id.clone()));
                out.push(b);
            }
        }
        return out;
    };

    let mut edges: Vec<&GEdge> = graph.edges.iter().collect();
    edges.sort_by(|a, b| a.id.cmp(&b.id));
    for e in edges {
        if !label_ok(&edge_pat.label, &e.label) {
            continue;
        }
        // Outgoing (a)-[:R]->(b): a=src, b=dst. Incoming (a)<-[:R]-(b): the edge
        // runs b -> a, so a=dst, b=src.
        let (left_id, right_id) = match edge_pat.dir {
            Dir::Outgoing => (&e.src, &e.dst),
            Dir::Incoming => (&e.dst, &e.src),
        };
        let (Some(ln), Some(rn)) = (graph.node(left_id), graph.node(right_id)) else {
            continue;
        };
        if !label_ok(&pat.left.label, &ln.label) || !label_ok(&right.label, &rn.label) {
            continue;
        }
        let mut b = Binding::new();
        b.insert(pat.left.var.clone(), Ref::Node(ln.id.clone()));
        b.insert(right.var.clone(), Ref::Node(rn.id.clone()));
        if let Some(ev) = &edge_pat.var {
            b.insert(ev.clone(), Ref::Edge(e.id.clone()));
        }
        out.push(b);
    }
    out
}

fn resolve_field(
    var: &str,
    path: &Option<String>,
    b: &Binding,
    graph: &Graph,
) -> Result<QValue, String> {
    let Some(r) = b.get(var) else {
        return Err(format!("unbound variable {var}"));
    };
    match (r, path) {
        (Ref::Node(id), None) => Ok(QValue::Node(id.clone())),
        (Ref::Edge(id), None) => Ok(QValue::Edge(id.clone())),
        (Ref::Node(id), Some(p)) => Ok(graph
            .node(id)
            .and_then(|n| n.props.get(p).cloned())
            .unwrap_or(QValue::Null)),
        (Ref::Edge(id), Some(p)) => Ok(graph
            .edge(id)
            .and_then(|e| e.props.get(p).cloned())
            .unwrap_or(QValue::Null)),
    }
}

fn resolve_operand(
    op: &Operand,
    b: &Binding,
    graph: &Graph,
    params: &BTreeMap<String, QValue>,
) -> Result<QValue, String> {
    match op {
        Operand::Field { var, path } => resolve_field(var, path, b, graph),
        Operand::Lit(l) => Ok(lit_value(l)),
        Operand::Param(name) => Ok(params.get(name).cloned().unwrap_or(QValue::Null)),
    }
}

fn eval_expr(
    e: &Expr,
    b: &Binding,
    graph: &Graph,
    params: &BTreeMap<String, QValue>,
) -> Result<bool, String> {
    Ok(match e {
        Expr::Or(l, r) => eval_expr(l, b, graph, params)? || eval_expr(r, b, graph, params)?,
        Expr::And(l, r) => eval_expr(l, b, graph, params)? && eval_expr(r, b, graph, params)?,
        Expr::Not(x) => !eval_expr(x, b, graph, params)?,
        Expr::Cmp(l, op, r) => cmp_predicate(
            &resolve_operand(l, b, graph, params)?,
            *op,
            &resolve_operand(r, b, graph, params)?,
        ),
        Expr::IsNull(x) => matches!(resolve_operand(x, b, graph, params)?, QValue::Null),
        Expr::IsNotNull(x) => !matches!(resolve_operand(x, b, graph, params)?, QValue::Null),
    })
}

fn project(item: &Item, b: &Binding, graph: &Graph) -> Result<QValue, String> {
    resolve_field(&item.var, &item.path, b, graph)
}

/// Evaluate a query, returning ordered result rows.
pub fn eval(
    query: &Query,
    graph: &Graph,
    params: &BTreeMap<String, QValue>,
) -> Result<Vec<Vec<QValue>>, String> {
    // MATCH → WHERE.
    let mut bindings = bind(query, graph);
    if let Some(filter) = &query.filter {
        let mut kept = Vec::new();
        for b in bindings {
            if eval_expr(filter, &b, graph, params)? {
                kept.push(b);
            }
        }
        bindings = kept;
    }

    // Sort key per binding: the ORDER BY value + its owning node id as final
    // tiebreak; with no ORDER BY, order by the bound ids in pattern order.
    let ids = |b: &Binding| -> Vec<String> {
        b.values()
            .map(|r| match r {
                Ref::Node(id) | Ref::Edge(id) => id.clone(),
            })
            .collect()
    };
    let mut rows: Vec<(QValue, Vec<String>, Binding)> = Vec::new();
    for b in bindings {
        let key = match &query.order_by {
            Some((item, _)) => project(item, &b, graph)?,
            None => QValue::Null,
        };
        rows.push((key, ids(&b), b));
    }
    let descending = query.order_by.as_ref().map(|(_, d)| *d).unwrap_or(false);
    rows.sort_by(|a, b| {
        let mut o = total_order(&a.0, &b.0);
        if descending {
            o = o.reverse();
        }
        o.then_with(|| a.1.cmp(&b.1)) // id tiebreak is always ascending, stable
    });

    // LIMIT.
    if let Some(limit) = query.limit {
        rows.truncate(limit as usize);
    }

    // RETURN projection.
    let mut out = Vec::new();
    for (_, _, b) in &rows {
        let mut row = Vec::new();
        for item in &query.items {
            row.push(project(item, b, graph)?);
        }
        out.push(row);
    }
    Ok(out)
}
