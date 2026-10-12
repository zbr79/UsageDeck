const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]'])

/**
 * Only loopback Host headers are served. Anything else (e.g. a DNS-rebound
 * attacker domain resolving to 127.0.0.1) is rejected.
 */
export function isAllowedHost(host: string | null | undefined): boolean {
  if (!host) return false
  const value = host.trim().toLowerCase()
  const hostname = value.startsWith('[')
    ? value.slice(0, value.indexOf(']') + 1)
    : value.split(':')[0]
  return LOOPBACK_HOSTS.has(hostname)
}
