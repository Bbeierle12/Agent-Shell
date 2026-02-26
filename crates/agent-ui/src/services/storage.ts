/**
 * Persistent canvas storage using IndexedDB.
 *
 * Falls back to localStorage for browsers without IndexedDB support,
 * but IndexedDB is preferred because it supports gigabytes of data
 * (vs ~5 MB for localStorage) and avoids quota-exceeded errors when
 * agent conversations generate large outputs.
 */

import { openDB, type IDBPDatabase } from 'idb'

const DB_NAME = 'agent_canvas'
const DB_VERSION = 1
const STORE_NAME = 'state'
const STATE_KEY = 'canvas_state'

// Legacy localStorage key (migrated on first IndexedDB read).
const LEGACY_KEY = 'agent_canvas_state'

let dbPromise: Promise<IDBPDatabase> | null = null

function getDB(): Promise<IDBPDatabase> {
  if (!dbPromise) {
    dbPromise = openDB(DB_NAME, DB_VERSION, {
      upgrade(db) {
        if (!db.objectStoreNames.contains(STORE_NAME)) {
          db.createObjectStore(STORE_NAME)
        }
      },
    })
  }
  return dbPromise
}

/**
 * Load the canvas state.
 *
 * 1. Tries IndexedDB first.
 * 2. If empty, checks localStorage (legacy migration).
 * 3. Migrates localStorage data into IndexedDB and removes the old key.
 */
export async function loadCanvasState<T>(): Promise<T | null> {
  try {
    const db = await getDB()
    const value = await db.get(STORE_NAME, STATE_KEY) as T | undefined
    if (value) return value

    // Migrate from localStorage if present.
    const legacy = localStorage.getItem(LEGACY_KEY)
    if (legacy) {
      const parsed = JSON.parse(legacy) as T
      await db.put(STORE_NAME, parsed, STATE_KEY)
      localStorage.removeItem(LEGACY_KEY)
      return parsed
    }
  } catch {
    // IndexedDB unavailable — fall back to localStorage.
    try {
      const raw = localStorage.getItem(LEGACY_KEY)
      if (raw) return JSON.parse(raw) as T
    } catch { /* ignore */ }
  }
  return null
}

/**
 * Save the canvas state.
 *
 * Writes to IndexedDB (with localStorage fallback).
 */
export async function saveCanvasState<T>(state: T): Promise<void> {
  try {
    const db = await getDB()
    await db.put(STORE_NAME, state, STATE_KEY)
  } catch {
    // Fallback to localStorage (best-effort, may throw on quota).
    try {
      localStorage.setItem(LEGACY_KEY, JSON.stringify(state))
    } catch { /* ignore quota errors */ }
  }
}
