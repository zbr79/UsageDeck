import { describe, expect, it } from 'vitest'
import { decryptKey, encryptKey, isEncryptedKey } from '@/lib/key-crypto'

describe('key crypto', () => {
  it('round-trips an API key and does not store the plaintext', () => {
    const secret = 'local-test-secret'
    const stored = encryptKey('sk-live-key', secret)
    expect(isEncryptedKey(stored)).toBe(true)
    expect(stored).not.toContain('sk-live-key')
    expect(decryptKey(stored, secret)).toBe('sk-live-key')
  })

  it('leaves an already-plaintext value unchanged when it is not encrypted', () => {
    expect(decryptKey('sk-old', 'secret')).toBe('sk-old')
  })

  it('rejects a ciphertext encrypted with a different secret', () => {
    const stored = encryptKey('sk-live-key', 'one')
    expect(() => decryptKey(stored, 'two')).toThrow()
  })
})
