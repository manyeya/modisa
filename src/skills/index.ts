// The shepherd skill: how an agent running in a pane drives the workspace. Installed by src/integrations
// into each agent that reads SKILL.md. `with { type: "text" }` is required — Bun's default .md loader
// renders HTML.
import SKILL from "./shepherd/SKILL.md" with { type: "text" };

export { SKILL };
