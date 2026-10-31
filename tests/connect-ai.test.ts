// "Design with your AI" (apps/web/src/components/ConnectAi.tsx): which assistant a connection is, and the
// documented links and commands it hands out.
import { describe, expect, it } from 'vitest';
import { claudeInstallLink, clientOf, codexCommand, codexConfig, claudeCodeCommand, isLocalOrigin } from '../apps/web/src/components/ConnectAi';

describe('connect your AI', () => {
  it('tells assistants apart by verified domain first, then by name', () => {
    expect(clientOf({ name: 'Claude', domain: 'claude.ai' })).toBe('claude');
    expect(clientOf({ name: 'Claude Code', domain: 'claude.ai' })).toBe('claude');
    expect(clientOf({ name: 'ChatGPT', domain: 'chatgpt.com' })).toBe('chatgpt');
    expect(clientOf({ name: 'Codex CLI', domain: null })).toBe('codex');
    expect(clientOf({ name: 'Claude', domain: null })).toBe('claude');
    // A verified domain wins over a name that claims to be someone else.
    expect(clientOf({ name: 'ChatGPT', domain: 'evil.example' })).toBeNull();
    expect(clientOf({ name: 'Zapier', domain: null })).toBeNull();
  });

  it('builds the documented links and commands', () => {
    const url = 'https://design.example.com/mcp';
    const link = new URL(claudeInstallLink(url));
    expect(link.origin + link.pathname).toBe('https://claude.ai/customize/connectors');
    expect(Object.fromEntries(link.searchParams)).toEqual({ modal: 'add-custom-connector', connectorName: 'Pinselo', connectorUrl: url });
    expect(claudeInstallLink(url)).toContain('connectorUrl=https%3A%2F%2Fdesign.example.com%2Fmcp');
    expect(claudeCodeCommand(url)).toBe(`claude mcp add --transport http pinselo ${url}`);
    expect(codexCommand(url)).toBe(`codex mcp add pinselo --url ${url} && codex mcp login pinselo`);
    expect(codexConfig(url)).toBe(`[mcp_servers.pinselo]\nurl = "${url}"`);
  });

  it('knows when hosted assistants can’t reach the server', () => {
    for (const h of ['localhost', '127.0.0.1', '192.168.1.20', '10.0.0.5', 'mac.local']) expect(isLocalOrigin(h), h).toBe(true);
    for (const h of ['design.example.com', '172.32.0.1']) expect(isLocalOrigin(h), h).toBe(false);
  });
});
