// Built-in colour themes for the TUI.
import type { Config } from "./config";

export type Theme = { bg: string; bar: string; fg: string; dim: string; border: string; focus: string; accent: string; warn: string; blocked: string; working: string; done: string; idle: string };

export const THEMES: Record<string, Theme> = {
  ion: { bg: "#090f1b", bar: "#101b2c", fg: "#d9e7f5", dim: "#8295ad", border: "#293c54", focus: "#5ee7ef", accent: "#b39aff", warn: "#f0c674", blocked: "#ff7f96", working: "#5ee7ef", done: "#a5efb5", idle: "#8295ad" },
  tokyonight: { bg: "#1a1b26", bar: "#16161e", fg: "#c0caf5", dim: "#565f89", border: "#3b4261", focus: "#7aa2f7", accent: "#7aa2f7", warn: "#e0af68", blocked: "#f7768e", working: "#e0af68", done: "#7dcfff", idle: "#9ece6a" },
  "catppuccin-mocha": { bg: "#1e1e2e", bar: "#181825", fg: "#cdd6f4", dim: "#6c7086", border: "#45475a", focus: "#cba6f7", accent: "#cba6f7", warn: "#f9e2af", blocked: "#f38ba8", working: "#f9e2af", done: "#89b4fa", idle: "#a6e3a1" },
  gruvbox: { bg: "#282828", bar: "#1d2021", fg: "#ebdbb2", dim: "#928374", border: "#504945", focus: "#fabd2f", accent: "#fabd2f", warn: "#fe8019", blocked: "#fb4934", working: "#fabd2f", done: "#83a598", idle: "#b8bb26" },
  nord: { bg: "#2e3440", bar: "#242933", fg: "#eceff4", dim: "#616e88", border: "#434c5e", focus: "#88c0d0", accent: "#88c0d0", warn: "#ebcb8b", blocked: "#bf616a", working: "#ebcb8b", done: "#81a1c1", idle: "#a3be8c" },
  dracula: { bg: "#282a36", bar: "#21222c", fg: "#f8f8f2", dim: "#6272a4", border: "#44475a", focus: "#bd93f9", accent: "#bd93f9", warn: "#f1fa8c", blocked: "#ff5555", working: "#f1fa8c", done: "#8be9fd", idle: "#50fa7b" },
};

export const theme = (c: Config) => THEMES[c.theme] ?? THEMES.tokyonight!;
