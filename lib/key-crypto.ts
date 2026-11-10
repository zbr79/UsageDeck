import crypto from 'crypto'

const PREFIX = 'enc:v1:'

function aesKey(secret: string): Buffer {
  return crypto.createHash('sha256').update(secret).digest()
}

export function isEncryptedKey(value: string): boolean {
  return value.startsWith(PREFIX)
}

export function encryptKey(plain: string, secret: string): string {
  const iv = crypto.randomBytes(12)
  const cipher = crypto.createCipheriv('aes-256-gcm', aesKey(secret), iv)
  const data = Buffer.concat([cipher.update(plain, 'utf8'), cipher.final()])
  const tag = cipher.getAuthTag()
  return `${PREFIX}${iv.toString('base64url')}:${tag.toString('base64url')}:${data.toString('base64url')}`
}

export function decryptKey(stored: string, secret: string): string {
  if (!isEncryptedKey(stored)) return stored
  const body = stored.slice(PREFIX.length)
  const [ivPart, tagPart, dataPart] = body.split(':')
  if (!ivPart || !tagPart || !dataPart) {
    throw new Error('Stored API key is not valid encrypted data')
  }
  const decipher = crypto.createDecipheriv(
    'aes-256-gcm',
    aesKey(secret),
    Buffer.from(ivPart, 'base64url'),
  )
  decipher.setAuthTag(Buffer.from(tagPart, 'base64url'))
  return Buffer.concat([
    decipher.update(Buffer.from(dataPart, 'base64url')),
    decipher.final(),
  ]).toString('utf8')
}
