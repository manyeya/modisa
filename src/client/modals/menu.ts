// A small menu at (x, y): the searchable list, placed there, whose items also run on their own keys.
import type { App } from "../context";
import { list } from "./pick";

export type MenuOption = { name: string; key: string; action: string; danger?: boolean };

export function menu(app: App, title: string, options: MenuOption[], x: number, y: number): Promise<string | null> {
  return list(app, { title, at: { x, y }, width: 44, rows: 14, items: options.map((o) => ({ name: o.name, key: o.key, value: o.action, danger: o.danger })) });
}
