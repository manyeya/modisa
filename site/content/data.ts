// What the site shows that comes from the code itself, as the modisa binary reports it (`modisa __site-data`, built
// from this checkout): the agents it knows, every setting, the integrations, and the plugin directory from GitHub.
export type Agent = { id: string; name: string; launch: string };
export type Found = { name: string; repo: string; url: string; description: string; stars: number; updated: string; created: string; archived: boolean; install: string };
type Data = { agents: Agent[]; sample: string; integrations: Record<"lifecycle" | "session", string>; topic: string; plugins: Found[] | { error: string } };

export const DATA: Data = JSON.parse(await Bun.$`cargo run --release --quiet -- __site-data`.cwd(`${import.meta.dir}/../..`).text());
