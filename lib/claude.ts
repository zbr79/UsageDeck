import { promises as fs } from 'fs'
import path from 'path'
import os from 'os'
import { logWarn } from '@/lib/log'

const CLAUDE_JSON = path.join(os.homedir(), '.claude.json')
const CREDENTIALS = path.join(os.homedir(), '.claude', '.credentials.json')

export interface ClaudeUsage {
  email: string | null
  displayName: string | null
  signedIn: boolean
  subscribed: boolean
  subscriptionType: string | null
  billingType: string | null
  rateLimitTier: string | null
  orgUuid: string | null
}

export function parseClaudeUsage(
  mainJson?: Record<string, unknown> | null,
  credsJson?: Record<string, unknown> | null,
): ClaudeUsage {
  const oauth = (mainJson?.oauthAccount ?? null) as Record<string, unknown> | null
  const claudeAiOauth = (credsJson?.claudeAiOauth ?? null) as Record<string, unknown> | null
  const subType =
    typeof claudeAiOauth?.subscriptionType === 'string'
      ? claudeAiOauth.subscriptionType
      : null
  const tier =
    typeof claudeAiOauth?.rateLimitTier === 'string' ? claudeAiOauth.rateLimitTier : null
  const signedIn = Boolean(claudeAiOauth?.accessToken)
  const billingType = typeof oauth?.billingType === 'string' ? oauth.billingType : null
  const seatTier = oauth?.seatTier ?? null
  const subscribed =
    subType != null || (billingType != null && billingType !== 'none') || seatTier != null
  return {
    email: typeof oauth?.emailAddress === 'string' ? oauth.emailAddress : null,
    displayName: typeof oauth?.displayName === 'string' ? oauth.displayName : null,
    signedIn,
    subscribed,
    subscriptionType: subType,
    billingType,
    rateLimitTier: tier,
    orgUuid: typeof oauth?.organizationUuid === 'string' ? oauth.organizationUuid : null,
  }
}

let cache: { at: number; data: ClaudeUsage | null } | null = null

export async function getClaudeUsage(): Promise<ClaudeUsage | null> {
  if (cache && Date.now() - cache.at < 60_000) return cache.data
  try {
    let mainJson: Record<string, unknown> | null = null
    try {
      mainJson = JSON.parse(await fs.readFile(CLAUDE_JSON, 'utf8'))
    } catch {
      // Expected when Claude Code is not installed
      mainJson = null
    }
    let credsJson: Record<string, unknown> | null = null
    try {
      credsJson = JSON.parse(await fs.readFile(CREDENTIALS, 'utf8'))
    } catch {
      // Expected when Claude credentials file is absent
      credsJson = null
    }
    const data = parseClaudeUsage(mainJson, credsJson)
    cache = { at: Date.now(), data }
    return data
  } catch (err) {
    logWarn('claude', 'Failed to read Claude usage', err)
    cache = { at: Date.now(), data: null }
    return null
  }
}
