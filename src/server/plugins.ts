// Plugins: any executable from [[plugin]] in the config, started with $SHEPHERD_SOCKET set.
import type { Config } from "../config/config";

export function startPlugins(cfg: Config): { stop(): void } {
  const procs = cfg.plugin.map((pl) =>
    Bun.spawn([Bun.env.SHELL || "/bin/sh", "-lc", pl.run], { env: { ...Bun.env }, stdio: ["ignore", "inherit", "inherit"] }),
  );
  return { stop: () => procs.forEach((p) => p.kill()) };
}
