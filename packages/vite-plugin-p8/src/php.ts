// What vite-plugin-p8 asks p8 to do: serve one web request, or render a
// template with props. Both are p8's own command lines; nothing here
// sets a PHP variable itself.
//
// A request: `p8 -S <host>:<port> --stdio <client> -t <root> -q`, the raw
// HTTP/1.1 request on stdin, the raw response on stdout. p8 runs it as
// `php -S` (the cli-server SAPI) runs a request off a socket, so $_SERVER,
// $_GET, $_POST, $_COOKIE, $_FILES, php://input, header() and
// http_response_code() are PHP's own.
//
// A render: `p8 -r <bootstrap> -- <file>`, the props as JSON on stdin.
// The bootstrap decodes them into the global `$props` (an array: JSON
// objects become associative arrays) and requires the file, so the
// template runs in the global scope with its own __FILE__ and __DIR__.

import { P8, type RunResult } from './runtime.ts';

/** Where p8 runs and what it may touch. */
export interface Host {
  p8: P8;
  /** The document root (read-only, at its own path). */
  root: string;
  /** More read-only directories (at their own paths). */
  readOnly: string[];
  /** A writable directory: TMPDIR, for uploads and session files. */
  tmpDir?: string;
  /** `-d name=value` settings for every run. */
  ini: Record<string, string>;
}

/** A web request as p8 reads it off a connection. */
export interface PhpRequest {
  method: string;
  /** The request target: path and query string. */
  url: string;
  /** Header names and values as the client sent them. */
  headers: [string, string][];
  body: Uint8Array;
  /** The server's address (SERVER_NAME, SERVER_PORT). */
  server: { host: string; port: number };
  /** The client's address (REMOTE_ADDR, REMOTE_PORT). */
  client: { address: string; port: number };
}

/** A response as p8 wrote it. */
export interface PhpResponse {
  status: number;
  headers: [string, string][];
  body: Buffer;
  /** p8's log (warnings it could not put in the page). */
  log: string;
}

const iniArgs = (ini: Record<string, string>) => Object.entries(ini).flatMap(([name, value]) => ['-d', `${name}=${value}`]);

const address = (host: string, port: number) => `${host.includes(':') ? `[${host}]` : host}:${port}`;

function env(host: Host): Record<string, string> {
  return host.tmpDir ? { TMPDIR: host.tmpDir } : {};
}

function run(host: Host, args: string[], stdin: Uint8Array): Promise<RunResult> {
  return host.p8.run({
    args,
    env: env(host),
    stdin,
    readOnly: [host.root, ...host.readOnly],
    writable: host.tmpDir ? [host.tmpDir] : [],
  });
}

/** The request as HTTP/1.1 bytes: its body whole, so its length is known. */
export function requestBytes(request: PhpRequest): Buffer {
  const hop = new Set(['content-length', 'transfer-encoding', 'connection', 'keep-alive']);
  let head = `${request.method} ${request.url} HTTP/1.1\r\n`;
  for (const [name, value] of request.headers) {
    if (!hop.has(name.toLowerCase())) head += `${name}: ${value}\r\n`;
  }
  if (request.body.length > 0 || !['GET', 'HEAD'].includes(request.method)) head += `Content-Length: ${request.body.length}\r\n`;
  head += 'Connection: close\r\n\r\n';
  return Buffer.concat([Buffer.from(head, 'latin1'), request.body]);
}

/** An HTTP/1.x response's status, headers and body. */
export function parseResponse(raw: Buffer): Omit<PhpResponse, 'log'> {
  const end = raw.indexOf('\r\n\r\n');
  if (end < 0) throw new Error(`p8 wrote no HTTP response: ${raw.toString('latin1').slice(0, 200)}`);
  const lines = raw.subarray(0, end).toString('latin1').split('\r\n');
  const status = Number(/^HTTP\/\d\.\d (\d{3})/.exec(lines[0])?.[1]);
  if (!status) throw new Error(`p8 wrote no status line: ${lines[0]}`);
  const headers: [string, string][] = [];
  for (const line of lines.slice(1)) {
    const colon = line.indexOf(':');
    if (colon > 0) headers.push([line.slice(0, colon).trim(), line.slice(colon + 1).trim()]);
  }
  return { status, headers, body: raw.subarray(end + 4) };
}

/** Serve one request with the cli-server SAPI, as `php -S -t root` would;
 * with `router`, as `php -S -t root router.php` would (that file runs for
 * the request, whatever its path). */
export async function serve(host: Host, request: PhpRequest, router?: string): Promise<PhpResponse> {
  const args = ['-S', address(request.server.host, request.server.port), '--stdio', address(request.client.address, request.client.port), '-t', host.root, '-q', ...iniArgs(host.ini), ...(router ? [router] : [])];
  const ran = await run(host, args, requestBytes(request));
  if (ran.stdout.length === 0) throw new Error(`p8 served no response (exit ${ran.status}): ${ran.stderr.toString()}`);
  return { ...parseResponse(ran.stdout), log: ran.stderr.toString() };
}

/** The bootstrap `render` runs: `$props` from stdin, then the template. */
export const RENDER_BOOTSTRAP = '$props = json_decode(stream_get_contents(STDIN), true); require $argv[1];';

/** Render `file` with `props` (JSON) as the global `$props`: what it prints. */
export async function render(host: Host, file: string, props: unknown): Promise<string> {
  const args = [...iniArgs(host.ini), '-r', RENDER_BOOTSTRAP, '--', file];
  const ran = await run(host, args, Buffer.from(JSON.stringify(props ?? null)));
  if (ran.status !== 0) {
    throw new Error(`p8 exited with ${ran.status} rendering ${file}:\n${ran.stdout.toString()}${ran.stderr.toString()}`);
  }
  return ran.stdout.toString();
}
