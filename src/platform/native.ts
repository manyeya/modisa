// libghostty-vt's native libraries, embedded so the compiled binary is self-contained.
// From source they're real files in node_modules; inside a compiled binary they live in /$bunfs
// and get copied out once, since dlopen needs a real path.
import { DIR } from "../core/paths";
import darwinArm64 from "../../node_modules/libghostty-vt/prebuilds/darwin-arm64/libghostty-vt.dylib" with { type: "file" };
import darwinArm64Shim from "../../node_modules/libghostty-vt/prebuilds/darwin-arm64/libghostty-vt-shim.dylib" with { type: "file" };
import linuxX64 from "../../node_modules/libghostty-vt/prebuilds/linux-x64-glibc/libghostty-vt.so" with { type: "file" };
import linuxX64Shim from "../../node_modules/libghostty-vt/prebuilds/linux-x64-glibc/libghostty-vt-shim.so" with { type: "file" };
import linuxArm64 from "../../node_modules/libghostty-vt/prebuilds/linux-arm64-glibc/libghostty-vt.so" with { type: "file" };
import linuxArm64Shim from "../../node_modules/libghostty-vt/prebuilds/linux-arm64-glibc/libghostty-vt-shim.so" with { type: "file" };

// ponytail: glibc only when compiled; musl users run from source (the package finds its own musl build)
const LIBS: Record<string, [lib: string, shim: string, ext: string]> = {
  "Darwin arm64": [darwinArm64, darwinArm64Shim, "dylib"],
  "Linux x86_64": [linuxX64, linuxX64Shim, "so"],
  "Linux aarch64": [linuxArm64, linuxArm64Shim, "so"],
};

export async function prepareNative() {
  if (!import.meta.path.startsWith("/$bunfs/") || Bun.env.GHOSTTY_VT_LIB) return;
  const host = (await Bun.$`uname -sm`.text()).trim();
  const libs = LIBS[host];
  if (!libs) throw new Error(`no bundled libghostty-vt for ${host}; run modisa from source`);
  const [lib, shim, ext] = libs;
  const dir = `${DIR}/lib/${Bun.hash(lib + shim + Bun.version).toString(36)}`;
  // the shim finds libghostty-vt next to itself, under the name it was linked against
  const names = ext === "so" ? ["libghostty-vt.so", "libghostty-vt.so.0", "libghostty-vt.so.0.0"] : ["libghostty-vt.dylib"];
  for (const n of names) if (!(await Bun.file(`${dir}/${n}`).exists())) await Bun.write(`${dir}/${n}`, Bun.file(lib));
  if (!(await Bun.file(`${dir}/libghostty-vt-shim.${ext}`).exists())) await Bun.write(`${dir}/libghostty-vt-shim.${ext}`, Bun.file(shim));
  Bun.env.GHOSTTY_VT_LIB = `${dir}/libghostty-vt.${ext}`;
  Bun.env.GHOSTTY_VT_SHIM_LIB = `${dir}/libghostty-vt-shim.${ext}`;
}
