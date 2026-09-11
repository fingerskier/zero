import type { ReactNode } from 'react'

export type StoreStatus = 'loading' | 'ready' | 'error'
export type SyncStatus = 'offline' | 'ready'

export type ZeroDbAdapter = 'auto' | 'indexeddb' | 'opfs'

export interface ZeroDbProviderProps {
  children?: ReactNode
  /** wasm-bindgen `ZeroDb` constructor from `zerodb-wasm`. */
  ZeroDb: unknown
  name?: string
  adapter?: ZeroDbAdapter
  /** Test injection — same as `openDurable`. */
  indexedDB?: unknown
  opfsRoot?: unknown
  getDirectory?: unknown
  /** Persist signed ops on `onChange` (default true). */
  persistOnChange?: boolean
  /** Rendered instead of children while opening when provided. */
  fallback?: ReactNode
}

export interface ZeroDbContextValue {
  status: StoreStatus
  db: unknown
  journal: unknown
  restored: boolean
  adapter: string | null
  opCount: number
  error: Error | null
  persist: () => Promise<void>
}

export interface QueryResult {
  rows: unknown[]
  error: Error | null
  ready: boolean
}

export interface GraphNode {
  id: string
  label: string
  deleted: boolean
  props: Record<string, unknown>
}

export interface MutateFn {
  (fn: (db: any) => any): Promise<any>
  createNode(label: string): Promise<string>
  deleteNode(node: string): Promise<string>
  setLww(node: string, key: string, value: string): Promise<string>
  counterInc(node: string, key: string, n?: number): Promise<string>
  counterDec(node: string, key: string, n?: number): Promise<string>
  gcounterInc(node: string, key: string, n?: number): Promise<string>
  setAdd(node: string, key: string, value: string): Promise<string>
  setRemove(node: string, key: string, value: string): Promise<string>
  flagEnable(node: string, key: string): Promise<string>
  flagDisable(node: string, key: string): Promise<string>
  persist(): Promise<void>
}

export function ZeroDbProvider(props: ZeroDbProviderProps): ReactNode
export function useZeroDb(): ZeroDbContextValue
export function useQuery(query: string, params?: unknown): QueryResult
export function useNode(id: string | null | undefined): GraphNode | null
export function useMutation(): MutateFn
export function useSyncStatus(): SyncStatus
