Foundational Distributed Systems & Clocks

Lamport, L. (1978). “Time, Clocks, and the Ordering of Events in a Distributed System.” Communications of the ACM. (Causality, happened-before relation — foundational for any oplog/causal graph.)
Kulkarni, S., Demirbas, M., et al. (2014). “Logical Physical Clocks and Consistent Snapshots in Globally Distributed Databases.” Michigan State / Buffalo tech report (also appeared in Springer). (Direct source for Hybrid Logical Clocks / HLC — the exact mechanism chosen to replace wall-clock/HAM.)

CRDT Theory

Shapiro, M., Preguiça, N., Baquero, C., & Zawirski, M. (2011). “Conflict-free Replicated Data Types.” Stabilization, Safety, and Security of Distributed Systems (SSS 2011); also INRIA RR-7687. (The seminal paper defining state-based and operation-based CRDTs, convergence, SEC.)
Preguiça, N., Baquero, C., & Shapiro, M. (2018). “Conflict-free Replicated Data Types (CRDTs).” arXiv:1805.06358 (or the 2018 survey/update). (Modern overview of the field.)

Local-First Software & Practical CRDTs

Kleppmann, M., Wiggins, A., van Hardenberg, P., & McGranaghan, M. (2019). “Local-First Software: You Own Your Data, in spite of the Cloud.” Onward! 2019 / ACM. (The manifesto and practical experience that frames the entire local-first movement; discusses CRDTs as enabling technology.)
Kleppmann’s related work: “CRDTs: The Hard Parts,” convergence vs. consensus talks, and Automerge research papers (JSON CRDTs, interleaving anomalies, etc.). These surface real-world pitfalls you are already trying to avoid.

Merkle Structures & Authenticated Sync

Merkle, R. C. (1987). “A Digital Signature Based on a Conventional Encryption Function.” (Original Merkle tree paper — basis for your Merkle sync tree and tamper-evidence.)
General authenticated data structures literature and IPFS whitepaper / Git design docs (Merkle DAGs for efficient divergence detection and delta sync).

Property Graphs & Graph Data Models

Rodriguez, M. A., & Neubauer, P. (or standard property graph references such as Neo4j’s model papers). The property graph abstraction (nodes + first-class edges + properties on both) is well-established; cite to justify why it is superior to GunDB’s flat node+link model for your use cases.

Practical CRDT Systems (Storage Layer Inspiration)

vlcn-io/cr-sqlite (GitHub + vlcn.io/docs). “Convergent, Replicated SQLite” — multi-writer CRDT/causal-log extension for SQLite. Directly relevant to your zerodb-sqlite adapter and column-level CRDT approach.

Formal Verification & Correctness

Gomes, V., et al. (work on formal verification of CRDTs, referenced in Kleppmann’s local-first paper). Supports your Lean 4 proof goal in M5.
General dependent type theory / Lean 4 resources for specifying CRDT convergence and causal stability invariants.

Cryptography & Security (for the trust model)

Bernstein, D. J., et al. papers on Ed25519 / EdDSA (standard, high-assurance choice you already made).
Capability-based security literature (e.g., Levy’s Capability-Based Computer Systems or modern treatments in Tahoe-LAFS, Macaroons, or Matrix) for the datastore-membership capabilities design.

Additional suggestions for the bibliography

GunDB documentation or Mark Nadal’s writings (to explicitly position the successor relationship and the specific flaws being fixed).
CRDT.tech paper list (curated bibliography maintained by the community).
RFC 9562 (UUIDv7 — you already use sortable UUIDv7 timestamps).
CBOR RFC (for your canonical deterministic serialization).