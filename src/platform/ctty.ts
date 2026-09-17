// `modisa __pty-exec <cmd…>`: make the PTY our controlling terminal, then become <cmd>.
// Bun.spawn({ terminal, detached }) gives the child a new session with the PTY on stdio but doesn't
// claim it as the controlling tty; bash claims it itself, zsh doesn't — and without one there's no
// job control, ^C, or foreground process group (which agent detection reads). So: TIOCSCTTY, execvp.
import { dlopen, FFIType, ptr, suffix } from "bun:ffi";

export function execWithTty(argv: string[]): never {
  const mac = suffix === "dylib";
  const libc = dlopen(mac ? "/usr/lib/libSystem.B.dylib" : "libc.so.6", {
    ioctl: { args: [FFIType.i32, FFIType.u64], returns: FFIType.i32 },
    execvp: { args: [FFIType.ptr, FFIType.ptr], returns: FFIType.i32 },
  });
  libc.symbols.ioctl(0, mac ? 0x20007461 : 0x540e); // TIOCSCTTY on stdin (the PTY)
  const strings = argv.map((a) => new TextEncoder().encode(a + "\0"));
  const pointers = new BigUint64Array(strings.length + 1); // NULL-terminated char*[]
  strings.forEach((s, i) => (pointers[i] = BigInt(ptr(s))));
  libc.symbols.execvp(strings[0]!, pointers);
  throw new Error(`could not start ${argv[0]}`); // execvp only returns on failure
}
