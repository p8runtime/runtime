// The p8 command, built for wasm32-wasip1, run under Node's WASI (node:wasi).
//
// The WebAssembly module is compiled once; every run instantiates it again,
// because a WASI command runs its `main` once per instance (`_start`), and
// p8 serves one script or one request per `main`, as `php` does per
// process.
//
// Files: each preopened directory is mapped at its own absolute path, so
// __FILE__, __DIR__, include paths and file names are the ones the native
// p8 sees. node:wasi grants a preopen every right; the directories given
// as `readOnly` are made read-only here, at the WASI calls, with EROFS
// (PHP's "Read-only file system").
//
// stdin, stdout and stderr are this run's buffers, not the Node process's.

import { readFile } from 'node:fs/promises';
import { WASI } from 'node:wasi';

/** One run of p8: its command line, environment, stdin and directories. */
export interface RunOptions {
  /** p8's arguments (after the program name). */
  args: string[];
  /** The environment the script sees (none of Node's is passed). */
  env?: Record<string, string>;
  /** What the script reads from stdin. */
  stdin?: Uint8Array;
  /** Directories the script may read but never change. */
  readOnly?: string[];
  /** Directories the script may also write (a temporary directory). */
  writable?: string[];
}

/** What a run printed and how it ended. */
export interface RunResult {
  status: number;
  stdout: Buffer;
  stderr: Buffer;
}

// WASI preview1 numbers (wasi_snapshot_preview1).
const ERRNO_SUCCESS = 0;
const ERRNO_ROFS = 69;
const OFLAGS_CREAT = 1;
const OFLAGS_TRUNC = 8;
const FDFLAGS_APPEND = 1;
const RIGHT_FD_WRITE = 1n << 6n;
const RIGHT_FD_ALLOCATE = 1n << 8n;
const RIGHT_FD_FILESTAT_SET_SIZE = 1n << 22n;
const RIGHTS_THAT_WRITE =
  RIGHT_FD_WRITE |
  RIGHT_FD_ALLOCATE |
  RIGHT_FD_FILESTAT_SET_SIZE |
  (1n << 9n) | // path_create_directory
  (1n << 10n) | // path_create_file
  (1n << 11n) | // path_link_source
  (1n << 12n) | // path_link_target
  (1n << 16n) | // path_rename_source
  (1n << 17n) | // path_rename_target
  (1n << 19n) | // path_filestat_set_size
  (1n << 20n) | // path_filestat_set_times
  (1n << 23n) | // fd_filestat_set_times
  (1n << 24n) | // path_symlink
  (1n << 25n) | // path_remove_directory
  (1n << 26n); // path_unlink_file

type Fn = (...args: any[]) => any;

/** p8.wasm, compiled. */
export class P8 {
  readonly module: WebAssembly.Module;

  constructor(module: WebAssembly.Module) {
    this.module = module;
  }

  /** Read and compile p8.wasm (once per process). */
  static async load(wasmPath: string): Promise<P8> {
    const bytes = await readFile(wasmPath);
    return new P8(await WebAssembly.compile(bytes));
  }

  /** Run p8 once, in a new instance. */
  async run(options: RunOptions): Promise<RunResult> {
    const readOnly = options.readOnly ?? [];
    const writable = options.writable ?? [];
    const preopens: Record<string, string> = {};
    for (const dir of [...readOnly, ...writable]) preopens[dir] = dir;
    const wasi = new WASI({
      version: 'preview1',
      args: ['p8', ...options.args],
      env: options.env ?? {},
      preopens,
      returnOnExit: true,
    });
    const base = (wasi.getImportObject() as { wasi_snapshot_preview1: Record<string, Fn> }).wasi_snapshot_preview1;
    let memory: WebAssembly.Memory | undefined;
    const view = () => new DataView(memory!.buffer);
    const bytes = () => new Uint8Array(memory!.buffer);

    // stdin, stdout, stderr: this run's buffers.
    const stdin = options.stdin ?? new Uint8Array(0);
    let stdinAt = 0;
    const out: Uint8Array[] = [];
    const err: Uint8Array[] = [];
    const iovecs = (ptr: number, len: number) => {
      const v = view();
      const list: [number, number][] = [];
      for (let i = 0; i < len; i++) list.push([v.getUint32(ptr + i * 8, true), v.getUint32(ptr + i * 8 + 4, true)]);
      return list;
    };

    // The read-only directories: node:wasi numbers preopens from 3 in the
    // order given, and every descriptor opened under one is read-only too.
    const locked = new Set<number>(readOnly.map((_, i) => 3 + i));
    const refuse = (...fds: number[]) => fds.some((fd) => locked.has(fd));

    const imports: Record<string, Fn> = {
      ...base,
      fd_write(fd: number, iovs: number, iovsLen: number, written: number) {
        if (fd === 1 || fd === 2) {
          let n = 0;
          for (const [buf, len] of iovecs(iovs, iovsLen)) {
            (fd === 1 ? out : err).push(bytes().slice(buf, buf + len));
            n += len;
          }
          view().setUint32(written, n, true);
          return ERRNO_SUCCESS;
        }
        if (refuse(fd)) return ERRNO_ROFS;
        return base.fd_write(fd, iovs, iovsLen, written);
      },
      fd_read(fd: number, iovs: number, iovsLen: number, read: number) {
        if (fd === 0) {
          let n = 0;
          for (const [buf, len] of iovecs(iovs, iovsLen)) {
            const chunk = stdin.subarray(stdinAt, stdinAt + len);
            bytes().set(chunk, buf);
            stdinAt += chunk.length;
            n += chunk.length;
            if (chunk.length < len) break;
          }
          view().setUint32(read, n, true);
          return ERRNO_SUCCESS;
        }
        return base.fd_read(fd, iovs, iovsLen, read);
      },
      path_open(dirfd: number, dirflags: number, path: number, pathLen: number, oflags: number, rightsBase: bigint, rightsInheriting: bigint, fdflags: number, opened: number) {
        if (!locked.has(dirfd)) return base.path_open(dirfd, dirflags, path, pathLen, oflags, rightsBase, rightsInheriting, fdflags, opened);
        if (oflags & (OFLAGS_CREAT | OFLAGS_TRUNC) || fdflags & FDFLAGS_APPEND || rightsBase & (RIGHT_FD_WRITE | RIGHT_FD_ALLOCATE | RIGHT_FD_FILESTAT_SET_SIZE)) return ERRNO_ROFS;
        const errno = base.path_open(dirfd, dirflags, path, pathLen, oflags, rightsBase & ~RIGHTS_THAT_WRITE, rightsInheriting & ~RIGHTS_THAT_WRITE, fdflags, opened);
        if (errno === ERRNO_SUCCESS) locked.add(view().getUint32(opened, true));
        return errno;
      },
      fd_close(fd: number) {
        const errno = base.fd_close(fd);
        if (errno === ERRNO_SUCCESS) locked.delete(fd);
        return errno;
      },
      fd_renumber(from: number, to: number) {
        const errno = base.fd_renumber(from, to);
        if (errno === ERRNO_SUCCESS) {
          if (locked.has(from)) locked.add(to);
          else locked.delete(to);
          locked.delete(from);
        }
        return errno;
      },
    };
    // The calls that change what a directory holds or a file's size or times.
    const changes: Record<string, (args: number[]) => number[]> = {
      path_create_directory: (a) => [a[0]],
      path_remove_directory: (a) => [a[0]],
      path_unlink_file: (a) => [a[0]],
      path_filestat_set_times: (a) => [a[0]],
      path_rename: (a) => [a[0], a[3]],
      path_link: (a) => [a[0], a[4]],
      path_symlink: (a) => [a[2]],
      fd_filestat_set_times: (a) => [a[0]],
      fd_filestat_set_size: (a) => [a[0]],
      fd_allocate: (a) => [a[0]],
    };
    for (const [name, fds] of Object.entries(changes)) {
      imports[name] = (...args: number[]) => (refuse(...fds(args)) ? ERRNO_ROFS : base[name](...args));
    }

    const instance = await WebAssembly.instantiate(this.module, { wasi_snapshot_preview1: imports });
    memory = instance.exports.memory as WebAssembly.Memory;
    let status: number;
    try {
      status = wasi.start(instance);
    } catch (e) {
      // A trap (a Rust panic aborts): what p8 printed says why.
      throw new Error(`p8 ${options.args.join(' ')} trapped: ${e}\n${Buffer.concat(err).toString()}${Buffer.concat(out).toString()}`, { cause: e });
    }
    return { status, stdout: Buffer.concat(out), stderr: Buffer.concat(err) };
  }
}
