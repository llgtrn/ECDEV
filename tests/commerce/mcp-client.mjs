// ECDEV-owned minimal MCP client over Streamable HTTP: JSON-RPC 2.0 POSTs, a JSON or single-event SSE
// reply, and the session header when the server issues one. Only what the conformance smoke needs.
export class Client {
  constructor(info) { this.info = info; this.id = 0; this.session = null; this.url = null; }
  async connect(url) {
    this.url = url;
    await this.rpc('initialize', { protocolVersion: '2025-06-18', capabilities: {}, clientInfo: this.info });
    await this.post({ jsonrpc: '2.0', method: 'notifications/initialized' });
  }
  async post(body) {
    const headers = { 'content-type': 'application/json', accept: 'application/json, text/event-stream', 'mcp-protocol-version': '2025-06-18' };
    if (this.session) headers['mcp-session-id'] = this.session;
    const res = await fetch(this.url, { method: 'POST', headers, body: JSON.stringify(body) });
    const session = res.headers.get('mcp-session-id');
    if (session) this.session = session;
    return res;
  }
  async rpc(method, params) {
    const id = ++this.id;
    const res = await this.post({ jsonrpc: '2.0', id, method, params });
    if (!res.ok) throw new Error(`${method}: HTTP ${res.status}`);
    const text = await res.text();
    const payload = (res.headers.get('content-type') || '').includes('text/event-stream')
      ? JSON.parse(text.split('\n').filter((l) => l.startsWith('data:')).map((l) => l.slice(5).trim()).filter(Boolean).pop())
      : JSON.parse(text);
    if (payload.error) throw new Error(`${method}: ${payload.error.message}`);
    return payload.result;
  }
  listTools() { return this.rpc('tools/list', {}); }
  callTool(params) { return this.rpc('tools/call', params); }
  readResource(params) { return this.rpc('resources/read', params); }
  async close() {
    if (this.session) await fetch(this.url, { method: 'DELETE', headers: { 'mcp-session-id': this.session } }).catch(() => {});
  }
}
