// modisa-plugin.ts: modisa's client library for plugins (protocol 1). No dependencies.
// `modisa plugin new` copies it into each plugin; refresh a plugin's copy with `modisa plugin sdk > modisa-plugin.ts`.
//
// It handles the protocol so a plugin only writes its own logic: newline-delimited JSON-RPC framing, request ids,
// errors with modisa's stable codes, binding to the host (hello) with actions, and starting a subscription with no
// gap and no duplicates (subscribe). When the session's socket closes, `closed` resolves: exit then, because the next
// server starts the plugin again. runPlugin does all of that.

export const SDK_VERSION = 11;
export const PROTOCOL = 1;

export type AgentState = "working" | "blocked" | "done" | "idle";
// A pane: `id` (like p3) plus `instance`, unique to the process it was started with (ids come back after a restart).
// `agent` is what modisa's detection observed: its screen, or its integration's report. No `agent` means no agent
// is detected, not that there isn't one.
export type Pane = {
  id: string;
  instance: string;
  name?: string;
  title: string;
  terminalTitle?: string; // the title the program last set, e.g. an agent's task: a named pane's `title` stays its name
  cwd: string;
  command?: string;
  status: "running" | "exited";
  exitCode?: number;
  agent?: { harness: string; state: AgentState; source: "hook" | "screen" };
  workspace?: string;
  [field: string]: unknown;
};
// Every event has type, at (epoch ms), seq and epoch; `modisa plugin schema` has each type's fields.
export type Event = { type: string; at: number; seq: number; epoch: string; pane?: string; instance?: string; name?: string; harness?: string; from?: AgentState; to?: AgentState; [field: string]: unknown };
export type Snapshot = { protocol: number; epoch: string; seq: number; panes: Pane[] };
// An action gets its params and the call: `invocation` names it, and `signal` aborts if modisa gives up waiting
// (plugin.cancel, for that invocation only) or the connection goes. It's cooperative: an action that ignores it keeps
// running, the caller has already been told the outcome is unknown, and neither the abort nor a rejection proves
// that effects already started were undone.
// `target` is the pane the user took the action from (a menu entry, key or palette entry), kept apart from params and
// already checked by modisa to be that pane's current process. `link` is the URL the user Ctrl+clicked, when one of
// the manifest's links matched it (modisa checks the pattern); treat it as data, never as a command. `ui` says which of
// the plugin's views it came from, which element and what it held (ViewEvent: an input's text, a list's choice); it's
// what the user typed or chose, so treat it as data too.
export type Action = (params: Record<string, unknown>, call: { invocation?: string; signal: AbortSignal; target?: { pane: string; instance: string }; link?: string; ui?: ViewEvent }) => unknown;

// What a plugin shows in modisa's TUI (see Client.ui). Tones map to the user's theme: its text, dim, accent and
// warning colours, and its colours for the four agent states.
export type Tone = "fg" | "dim" | "accent" | "warn" | "working" | "blocked" | "done" | "idle";
// A piece of a sidebar row: text in a tone (bold if asked), or an agent's mark: `icon` is a built-in agent's id
// (claude-code, codex, gemini, …) and modisa draws its glyph in its brand colour.
export type Span = { text: string; tone?: Tone; bold?: boolean } | { icon: string };
// A row is `text` in one tone, or `spans` (then `text` may be left out). A row that focuses a pane names its instance
// too: clicking reaches that process, or tells the user it's gone.
export type SidebarRow = { text?: string; tone?: Tone; spans?: Span[]; action?: string; pane?: string; instance?: string };
export type MenuItem = { id: string; title: string; action: string };
export type UiState = {
  plugin: string;
  run: string;
  actions: { id: string; title: string; description?: string }[];
  status: { id: string; text: string; tone: Tone; action?: string }[];
  sidebar?: { title: string; rows: { text: string; tone: Tone; spans?: Span[]; action?: string; pane?: string; instance?: string }[] };
  badges: { pane: string; instance: string; text: string; tone: Tone }[];
  menu: MenuItem[];
  keys: { key: string; action?: string; pane?: string; description: string }[]; // as plugin.json declares them: each client binds them with its own config
  panes: { id: string; title: string; placement: "overlay" | "popup" | "split" | "tab" | "zoomed" }[];
  views?: (ViewOptions & { plugin: string; run: string; id: string; title: string; keys: ViewKey[]; root: ViewNode; rev: number })[]; // the views open now
};

// ---------- views: element trees modisa draws for a plugin (UI 3) ----------
// examples/plugins/VIEWS.md is the wire format, field by field; `modisa plugin schema` has it as JSON Schema. Elements
// are ratatui's own: constraint layouts, blocks around anything, ratatui's widgets, and a few modisa draws itself.

// Colours: a theme token ($accent…: it follows the user's theme, light or dark, so prefer it), #rrggbb, a name (red,
// light-blue, gray, …), an index ("0"–"255"), or reset.
export type Token = "fg" | "bg" | "bar" | "dim" | "border" | "focus" | "accent" | "warn" | "working" | "blocked" | "done" | "idle";
export type Color = `$${Token}` | `#${string}` | "reset" | (string & {});
export type Modifier = "bold" | "dim" | "italic" | "underlined" | "slow_blink" | "rapid_blink" | "reversed" | "hidden" | "crossed_out";
// "bold italic $accent on $bar" (modifiers, the foreground, `on` the background), or the same as an object, where
// `false` takes away a modifier the style under it has.
export type Style = string | ({ fg?: Color; bg?: Color; underline_color?: Color } & { [M in Modifier]?: boolean });
export type Align = "left" | "center" | "right";
// Text is spans in lines. A Span is a string, { text, style }, or { icon: "claude-code" } (an agent's mark in its colour).
// A Line is a Span, an array of Spans, or { spans, style, align }. A Text is a string (\n starts a line), an array of
// Lines, { lines, style, align }, or one Span or Line object. Escape sequences are taken out: show a program's coloured
// output with Text's `ansi`.
type SpanObject = { text: string; style?: Style } | { icon: string };
type LineObject = { spans: ViewSpan[]; style?: Style; align?: Align };
export type ViewSpan = string | SpanObject;
export type ViewLine = ViewSpan | ViewSpan[] | LineObject;
export type ViewText = string | SpanObject | LineObject | ViewLine[] | { lines: ViewLine[]; style?: Style; align?: Align };

// Where an element goes in its layout: 12 cells (Length), "30%", "1/3" (Ratio), ">=5" (Min), "<=20" (Max), "*" or "2*"
// (Fill: shares what's left, by weight). The functions below write them.
export type Constraint = number | `${number}` | `${number}%` | `${number}/${number}` | `>=${number}` | `<=${number}` | "*" | `${number}*`;
export const Length = (cells: number): Constraint => `${cells}`;
export const Min = (cells: number): Constraint => `>=${cells}`;
export const Max = (cells: number): Constraint => `<=${cells}`;
export const Percentage = (percent: number): Constraint => `${percent}%`;
export const Ratio = (a: number, b: number): Constraint => `${a}/${b}`;
// (Fill, below, is both: Fill(2) the constraint, Fill({ symbol }) the element.)

/** A span of text in a style: span("23%", "bold $warn"). */
export const span = (text: string | number, style?: Style): ViewSpan => (style === undefined ? String(text) : { text: String(text), style });
/** An agent's mark, in its colour: icon("claude-code"). */
export const icon = (agent: string): ViewSpan => ({ icon: agent });
/** A line of spans, with a style under them all and where it sits: line(["a ", span("b", "bold")], { align: "right" }). */
export const line = (spans: ViewSpan | ViewSpan[], options: { style?: Style; align?: Align } = {}): ViewLine => ({ spans: [spans].flat(), ...options });
/** A style from parts, leaving out those that are false: style("bold", late && "$warn", "on $bar"). */
export const style = (...parts: (string | false | null | undefined)[]): string => parts.filter(Boolean).join(" ");

export type BorderType = "plain" | "rounded" | "double" | "thick" | "light_double_dashed" | "heavy_double_dashed" | "light_triple_dashed" | "heavy_triple_dashed" | "light_quadruple_dashed" | "heavy_quadruple_dashed" | "quadrant_inside" | "quadrant_outside";
// A Block: a frame around an element (its `block`), or an element of its own (Block, with a `child`).
export type BlockFields = {
  borders?: "all" | "none" | ("top" | "right" | "bottom" | "left")[];
  border_type?: BorderType;
  border_style?: Style;
  title?: ViewLine;
  titles?: { content: ViewLine; position?: "top" | "bottom"; align?: Align }[];
  padding?: number | [vertical: number, horizontal: number] | [top: number, right: number, bottom: number, left: number];
  style?: Style;
  shadow?: boolean | { kind?: "dark_shade" | "medium_shade" | "light_shade" | "block" | "overlay"; offset?: [x: number, y: number]; style?: Style };
  merge?: "replace" | "exact" | "fuzzy"; // where borders overlap (a layout's negative spacing)
};
// What any element can have: `id` names it (anything interactive or stateful needs one, and it's what `focus` and its
// actions name); `size`, its constraint in its layout when the layout's `constraints` doesn't give one; a `block` around
// it; its area's `style`; and `hide_below`, a size under which it isn't drawn.
type Common = { id?: string; size?: Constraint; block?: BlockFields; style?: Style; hide_below?: { width?: number; height?: number } };
// The actions an element runs: `action` (Enter, a click, a press), `change` (the selection or text moved on).
type Acts = { action?: string; change?: string };
export type Flex = "legacy" | "start" | "end" | "center" | "space_between" | "space_around" | "space_evenly";
type HighlightSpacing = "always" | "when_selected" | "never";
export type Row = ViewCell[] | { cells: ViewCell[]; style?: Style; height?: number; top_margin?: number; bottom_margin?: number };
export type ViewCell = ViewText | { content: ViewText; style?: Style; span?: number };
export type Bar = { value: number; label?: ViewLine; text_value?: string; style?: Style; value_style?: Style };
export type Marker = "dot" | "block" | "bar" | "braille" | "half_block" | "quadrant" | "sextant" | "octant" | (string & {});
export type Dataset = { name?: ViewLine; data: [x: number, y: number][]; graph_type?: "line" | "scatter" | "bar" | "area"; marker?: Marker; style?: Style; fill_to?: number };
export type Axis = { title?: ViewLine; bounds?: [min: number, max: number]; labels?: ViewSpan[]; labels_align?: Align; style?: Style };
export type Shape =
  | { line: [x1: number, y1: number, x2: number, y2: number]; color?: Color }
  | { rectangle: [x: number, y: number, width: number, height: number]; color?: Color }
  | { circle: [x: number, y: number, radius: number]; color?: Color }
  | { points: [x: number, y: number][]; color?: Color }
  | { map: "low" | "high"; color?: Color }
  | { text: ViewLine; at: [x: number, y: number] }
  | { layer: true };
export type TreeItem = { id: string; text: ViewLine; children?: TreeItem[] };
export type ViewNode = Common &
  (
    | { type: "layout"; direction?: "vertical" | "horizontal"; constraints?: Constraint[]; flex?: Flex; spacing?: number; margin?: number | [vertical: number, horizontal: number]; children?: ViewNode[] }
    | { type: "text"; text?: ViewText; ansi?: string; align?: Align; wrap?: boolean | "trim"; scroll?: boolean | "bottom"; scrollbar?: boolean }
    | ({ type: "block"; child?: ViewNode } & BlockFields)
    | ({ type: "list"; items: (ViewLine | { content: ViewText; style?: Style })[]; selected?: number; highlight_style?: Style; highlight_symbol?: string; highlight_spacing?: HighlightSpacing; direction?: "top_to_bottom" | "bottom_to_top"; scroll_padding?: number } & Acts)
    | ({ type: "table"; header?: Row; footer?: Row; rows: Row[]; widths?: Constraint[]; column_spacing?: number; flex?: Flex; select?: "row" | "cell" | "column" | "none"; selected?: number | [row: number, column: number]; row_highlight_style?: Style; column_highlight_style?: Style; cell_highlight_style?: Style; highlight_symbol?: string; highlight_spacing?: HighlightSpacing } & Acts)
    | ({ type: "tabs"; titles: ViewLine[]; selected?: number; divider?: ViewSpan; padding?: [left: ViewSpan, right: ViewSpan]; highlight_style?: Style } & Acts)
    | { type: "gauge"; ratio?: number; percent?: number; label?: ViewSpan; gauge_style?: Style; unicode?: boolean }
    | { type: "line_gauge"; ratio?: number; percent?: number; label?: ViewSpan; filled_style?: Style; unfilled_style?: Style; filled_symbol?: string; unfilled_symbol?: string }
    | { type: "sparkline"; data: (number | null)[]; max?: number; direction?: "left_to_right" | "right_to_left"; bar_set?: "nine_levels" | "three_levels"; absent_symbol?: string; absent_style?: Style }
    | { type: "bar_chart"; groups?: { label?: ViewLine; bars: Bar[] }[]; data?: [label: string, value: number][]; direction?: "vertical" | "horizontal"; bar_width?: number; bar_gap?: number; group_gap?: number; max?: number; bar_style?: Style; value_style?: Style; label_style?: Style }
    | { type: "chart"; datasets: Dataset[]; x_axis?: Axis; y_axis?: Axis; legend?: "top_right" | "top_left" | "top" | "left" | "right" | "bottom" | "bottom_left" | "bottom_right" | "none" }
    | { type: "canvas"; x_bounds?: [min: number, max: number]; y_bounds?: [min: number, max: number]; marker?: Marker; background?: Color; shapes?: Shape[] }
    | { type: "calendar"; year: number; month: number; events?: Record<string, Style>; month_header?: Style | false; weekday_header?: Style | false; surrounding?: Style | false; default_style?: Style }
    | { type: "fill"; symbol?: string }
    | { type: "clear" }
    | { type: "code"; content: string; language?: string; line_numbers?: boolean | number; highlight?: number[]; wrap?: boolean; syntax_theme?: string }
    | ({ type: "diff"; diff: string; language?: string; view?: "unified" | "split"; line_numbers?: boolean; cursor?: boolean; marks?: number[] } & Acts)
    | { type: "markdown"; content: string }
    | { type: "big_text"; text: ViewText; pixel_size?: "full" | "half_height" | "half_width" | "quadrant" | "third_height" | "sextant" | "quarter_height" | "octant"; align?: Align }
    | { type: "image"; data: string; alt?: string; resize?: "fit" | "crop" | "scale" }
    | ({ type: "input"; value?: string; placeholder?: string; mask?: string } & Acts)
    | ({ type: "textarea"; value?: string; placeholder?: string; line_numbers?: boolean } & Acts)
    | ({ type: "tree"; items: TreeItem[]; open?: string[][]; selected?: string[]; highlight_style?: Style; highlight_symbol?: string; toggle?: string } & Acts)
    | { type: "button"; label?: ViewLine; action?: string; focus_style?: Style }
    | { type: "spinner"; label?: ViewLine; set?: "braille" | "dots" | "ascii" | "arrows" | "clock" | "circle" | "box" | "bounce" | "pulse" }
    | { type: "raster"; id: string; columns: number; rows: number; cells: string }
  );
// A key the view binds while it has focus: "j", "J", "enter", "S-tab", "C-s". Escape and Tab are modisa's.
export type ViewKey = { key: string; action: string; params?: Record<string, unknown>; description?: string };
// `focus`: the id of the element this update hands the keyboard to (the comment box just opened, say).
export type ViewOptions = { title?: string; placement?: "popup" | "overlay"; width?: ViewSize; height?: ViewSize; from?: { pane: string; instance: string }; keys?: ViewKey[]; close?: string; focus?: string };
// A view's size in cells, or a share of the terminal ("80%").
export type ViewSize = number | `${number}%`;
// What an element tells the action it runs (call.ui): the view, the element's id, what happened (`event`), and what the
// element holds. A list's or tabs' `index`; a table's `row` and `column`; a diff's `line` (which of its +, - and context
// lines, from 0), that line's `old` and `new` numbers (the one it has) and its `text`; an input's or textarea's `value`;
// a tree node's `path` (its ids from the root), and for `toggle` whether it's now `open`. A view's keys and its close
// action get just `view`. It's what the user typed or chose: treat it as data.
export type ViewEvent = { view: string; id?: string; event?: "action" | "change" | "toggle"; index?: number; row?: number; column?: number; line?: number; old?: number; new?: number; text?: string; value?: string; path?: string[]; open?: boolean };
export const VIEW_LIMITS = { perPlugin: 4, perSession: 8, elements: 5000, depth: 40, megabytes: 2, blitsPerSecond: 60 };

// Elements, as functions or JSX: `Layout({ direction: "horizontal", constraints: [Length(34), Fill(1)] }, List({…}), Diff({…}))`,
// or in a .tsx file `<Layout direction="horizontal"><List … /><Diff … /></Layout>`, with Bun's JSX as it comes or with
// `/** @jsx h */`. What goes inside one: a Layout's elements (a string becomes a Text), a Block's element (several are
// stacked in a Layout), a Text's or BigText's spans (a "\n" in a string starts a line), a Button's or Spinner's label,
// and the source of a Code, Markdown or Diff.
type Child = ViewNode | ViewSpan | LineObject | number | boolean | null | undefined | Child[];
type NodeOf<T extends ViewNode["type"]> = Extract<ViewNode, { type: T }>;
const INSIDE = { text: "text", big_text: "text", button: "label", spinner: "label", code: "content", markdown: "content", diff: "diff" } as const;
type Inside = typeof INSIDE;
type Props<T extends ViewNode["type"]> = (T extends keyof Inside ? Omit<NodeOf<T>, "type" | Inside[T]> & Partial<Pick<NodeOf<T>, Inside[T] & keyof NodeOf<T>>> : Omit<NodeOf<T>, "type" | "children">) & { children?: Child };
// JSX through React's automatic runtime (Bun's default) makes elements ({ type, props }), not nodes: this calls their
// components, so a view can be written either way.
function resolve(x: unknown): unknown {
  const e = x as { type?: unknown; props?: Record<string, unknown> } | null;
  if (!e || typeof e !== "object" || Array.isArray(e) || !e.props || typeof e.props !== "object") return x;
  if (typeof e.type === "function") return resolve(e.type(e.props));
  if (typeof e.type === "string") return el(e.type as ViewNode["type"])(e.props as never);
  return e.props.children; // a fragment
}
// what's inside an element, its components called, arrays flattened, and nothing (null, false) left out
const flat = (xs: unknown[]): any[] =>
  xs.flatMap((x) => {
    const r = resolve(x);
    return Array.isArray(r) ? flat(r) : r === null || r === undefined || typeof r === "boolean" ? [] : typeof r === "number" ? [String(r)] : [r];
  });
const nodes = (xs: unknown[]): ViewNode[] => xs.map((x) => (typeof x === "string" ? { type: "text", text: x } : (x as ViewNode)));
const one = (xs: ViewNode[]): ViewNode => (xs.length === 1 ? xs[0]! : { type: "layout", children: xs });
// a Text from spans: a "\n" in a string, or a { spans } line, starts a line
function lines(xs: (string | SpanObject | LineObject)[]): ViewText {
  if (xs.every((x) => typeof x === "string")) return xs.join("");
  const out: ViewLine[] = [];
  let at: ViewSpan[] = [];
  for (const x of xs) {
    if (typeof x === "string") {
      x.split("\n").forEach((part, i) => {
        if (i) out.push(at), (at = []);
        if (part) at.push(part);
      });
    } else if ("spans" in x) {
      if (at.length) out.push(at), (at = []);
      out.push(x);
    } else at.push(x);
  }
  if (at.length) out.push(at);
  return out;
}
const el = <T extends ViewNode["type"]>(type: T) => (props: Props<T> = {} as Props<T>, ...more: Child[]): NodeOf<T> => {
  const { children, ...rest } = props as Props<T> & { children?: Child };
  const xs = flat([children, ...more]);
  if (!xs.length) return { type, ...rest } as unknown as NodeOf<T>;
  const into = (INSIDE as Record<string, string>)[type];
  const inside =
    type === "layout" ? { children: nodes(xs) }
    : type === "block" ? { child: one(nodes(xs)) }
    : into === "text" ? { text: lines(xs) }
    : into === "label" ? { label: xs.every((x) => typeof x === "string") ? xs.join("") : xs }
    : into ? { [into]: xs.join("") }
    : undefined;
  if (!inside) throw new Error(`a ${type} has nothing inside it: give it its fields`);
  return { type, ...rest, ...inside } as unknown as NodeOf<T>;
};
export const Layout = el("layout"), Block = el("block"), Text = el("text"), List = el("list"), Table = el("table"), Tabs = el("tabs");
export const Gauge = el("gauge"), LineGauge = el("line_gauge"), Sparkline = el("sparkline"), BarChart = el("bar_chart"), Chart = el("chart"), Canvas = el("canvas"), Calendar = el("calendar");
export const Clear = el("clear"), Code = el("code"), Diff = el("diff"), Markdown = el("markdown"), BigText = el("big_text"), Image = el("image");
export const Input = el("input"), Textarea = el("textarea"), Tree = el("tree"), Button = el("button"), Spinner = el("spinner"), Raster = el("raster");
/** Fill(weight): the constraint that shares what's left, by weight. Fill({ symbol, style }): the element that paints its area. */
export function Fill(weight: number): Constraint;
export function Fill(props?: Props<"fill">): NodeOf<"fill">;
export function Fill(x?: number | Props<"fill">): Constraint | NodeOf<"fill"> {
  return typeof x === "number" ? (x === 1 ? "*" : `${x}*`) : el("fill")(x);
}
export const Fragment = (props: { children?: Child } | null, ...children: Child[]) => [props?.children, ...children];
/** The classic JSX factory, for `/** @jsx h *\/` and `/** @jsxFrag Fragment *\/` (or tsconfig's jsxFactory). */
export function h(type: ((props: any, ...children: Child[]) => unknown) | ViewNode["type"], props: Record<string, unknown> | null, ...children: Child[]): any {
  return typeof type === "function" ? type(props ?? {}, ...children) : el(type)((props ?? {}) as never, ...children);
}
export declare namespace h {
  namespace JSX {
    type Element = ViewNode | ViewSpan | Child[];
    interface ElementChildrenAttribute {
      children: unknown;
    }
    interface IntrinsicElements {
      [type: string]: Record<string, unknown>;
    }
  }
}

// A Raster's cells, painted by `paint(x, y)`: a character, or [character, fg, bg] with colours from `ink`. Width-1
// printable characters only (blocks, braille and box drawing are).
export const ink = {
  default: 0x01000000,
  tone: (t: Tone) => 0x02000000 | Math.max(0, (["fg", "dim", "accent", "warn", "working", "blocked", "done", "idle"] as Tone[]).indexOf(t)),
  rgb: (hex: string) => parseInt(hex.replace(/^#/, ""), 16) & 0xffffff,
};
export function rasterCells(columns: number, rows: number, paint: (x: number, y: number) => string | [string, number?, number?] | undefined) {
  const words = new Uint32Array(columns * rows * 3);
  for (let y = 0; y < rows; y++) {
    for (let x = 0; x < columns; x++) {
      const c = paint(x, y);
      const [ch, f, b] = typeof c === "string" ? [c] : (c ?? [" "]);
      const i = (y * columns + x) * 3;
      words[i] = ch.codePointAt(0) ?? 0x20;
      words[i + 1] = f ?? ink.default;
      words[i + 2] = b ?? ink.default;
    }
  }
  return Buffer.from(words.buffer).toString("base64");
}

export class ModisaError extends Error {
  constructor(message: string, readonly code: string) {
    super(message);
    this.name = "ModisaError";
  }
}

export class Client {
  private seq = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  private buf = "";
  private actions: Record<string, Action> = {};
  private running = new Map<string, AbortController>(); // invocation → its call's signal
  private listeners: ((e: Event) => void)[] = [];
  private cause?: Error;
  private onClosed!: (cause: Error) => void;
  /** Resolves, with the reason, once the connection to the session is gone. */
  readonly closed = new Promise<Error>((resolve) => (this.onClosed = resolve));

  constructor(private transport: { write(line: string): void; close(): void }) {}

  /** Bytes from the socket. */
  feed(chunk: string) {
    if (this.cause) return;
    this.buf += chunk;
    for (let nl; (nl = this.buf.indexOf("\n")) >= 0; ) {
      const line = this.buf.slice(0, nl);
      this.buf = this.buf.slice(nl + 1);
      let m: any;
      try {
        m = JSON.parse(line);
      } catch {
        continue; // a malformed line never ends the plugin
      }
      if (!m || typeof m !== "object") continue;
      if (m.id !== undefined && m.method === undefined) {
        const p = this.pending.get(m.id);
        if (!p) continue;
        this.pending.delete(m.id);
        if (m.error) p.reject(new ModisaError(String(m.error.message), m.error.data?.code ?? "error"));
        else p.resolve(m.result);
      } else if (m.method === "event" && m.params) for (const listen of this.listeners) listen(m.params);
      else if (m.method === "plugin.action" && m.id !== undefined) void this.answer(m.id, m.params ?? {});
      else if (m.method === "plugin.cancel") this.running.get(m.params?.invocation)?.abort(new Error("modisa stopped waiting for this action"));
    }
  }

  /** The connection is gone. */
  drop(cause = new Error("the session closed the connection")) {
    if (this.cause) return;
    this.cause = cause;
    this.buf = "";
    for (const p of this.pending.values()) p.reject(cause);
    this.pending.clear();
    for (const controller of this.running.values()) controller.abort(cause); // every action still running
    this.running.clear();
    this.onClosed(cause);
  }

  close() {
    this.transport.close();
    this.drop(new Error("closed by the plugin"));
  }

  request<T = unknown>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    if (this.cause) return Promise.reject(this.cause);
    const id = ++this.seq;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ jsonrpc: "2.0", id, method, params });
    });
  }

  /** This plugin's name, once hello has bound it. */
  name?: string;

  /**
   * Bind this connection to the plugin modisa started, offering actions to `modisa plugin run <name> <action>`,
   * the command palette, and the status segments, sidebar rows and menu entries that name them.
   */
  async hello(actions: Record<string, Action> = {}, token = Bun.env.MODISA_PLUGIN_TOKEN) {
    if (!token) throw new ModisaError("no $MODISA_PLUGIN_TOKEN: modisa starts plugins (modisa plugin link), not a shell", "usage");
    this.actions = actions;
    const bound = await this.request<{ name: string; protocol: number; session: string; epoch: string }>("plugin.hello", { token, actions: Object.keys(actions) });
    this.name = bound.name;
    return bound;
  }

  /**
   * What this plugin shows in modisa's TUI, drawn by modisa in the user's theme. Only after hello. Text is cleaned
   * of control characters and cut to length; an `action` must be one offered in hello; updates are rate-limited
   * (errors: rate_limited, no_such_action). Everything is cleared when the plugin stops.
   */
  readonly ui = {
    /** A status bar segment (up to 4); clicking it runs `action`. */
    status: (id: string, text: string, options: { tone?: Tone; action?: string } = {}) => this.request<UiState>("ui.status.set", { id, text, ...options }),
    clearStatus: (id: string) => this.request<UiState>("ui.status.clear", { id }),
    /** This plugin's sidebar section; a row runs its `action`, or focuses its `pane`. */
    sidebar: (title: string, rows: SidebarRow[]) => this.request<UiState>("ui.sidebar.set", { title, rows }),
    clearSidebar: () => this.request<UiState>("ui.sidebar.clear"),
    /** A label on a pane's border, for that pane's current process (`instance`) only. */
    badge: (pane: string, instance: string, text: string, tone?: Tone) => this.request<UiState>("ui.badge.set", { pane, instance, text, ...(tone && { tone }) }),
    clearBadge: (pane: string) => this.request<UiState>("ui.badge.clear", { pane }),
    /** Entries in the pane context menu; the action gets the pane it was opened on as `call.target` ({ pane, instance }). */
    menu: (items: MenuItem[]) => this.request<UiState>("ui.menu.set", { items }),
    /** A toast in every attached client (a system notification too, if the user has those on); a few per 10s. */
    toast: (text: string, options: { tone?: Tone; system?: boolean } = {}) => this.request<true>("ui.toast", { text, ...options }),
    /** Close this plugin's popup, if one is open. */
    closePopup: () => this.request<true>("ui.popup.close"),
    /** What this plugin shows now. */
    state: () => this.request<UiState>("ui.state", { plugin: this.name ?? "" }),
    /**
     * Open a view, or show something else in it: an element tree (see Layout, Block, List, … above) that modisa draws in
     * the user's theme, over everything (placement "popup", the default) or over the pane `from` (placement "overlay").
     * Updating it keeps what the user typed, chose and scrolled to in elements with the same `id`. The actions its
     * elements, `keys` and `close` name must be ones offered in hello. Limits: VIEW_LIMITS.
     */
    view: (id: string, root: ViewNode | Child, options: ViewOptions = {}) => this.request<{ id: string; rev: number; open: boolean }>("ui.view.set", { id, ...options, root: one(nodes(flat([root]))) }),
    closeView: (id: string) => this.request<boolean>("ui.view.close", { id }),
    /** Repaint the Raster with this id in an open view, in place (see rasterCells), at up to 60 a second: animation. */
    blit: (view: string, id: string, cells: string) => this.request<true>("ui.blit", { view, id, cells }),
  };

  /**
   * Start watching. `onSnapshot` gets every pane as of the moment the subscription starts. `onEvent` then gets each
   * later event once, in order, from the same server run, and never an event already reflected in the snapshot.
   * Handlers run one at a time. Events waiting for them are capped at `maxBacklog`: past it the connection is dropped
   * (a gap) rather than letting a stalled handler grow memory. A disconnect ends the stream (see `closed`); changes
   * that came and went while disconnected are lost, so don't promise a complete history.
   */
  async subscribe(handlers: { onSnapshot?: (snapshot: Snapshot) => unknown; onEvent: (event: Event) => unknown }, options: { output?: boolean; maxBacklog?: number } = {}) {
    const max = options.maxBacklog ?? 10_000;
    let ready = false;
    let epoch = "";
    let last = 0;
    let waiting = 0; // events handed to the queue and not handled yet
    const early: Event[] = [];
    let queue: Promise<unknown> = Promise.resolve();
    const overflow = () => {
      this.transport.close();
      this.drop(new ModisaError(`more than ${max} events waiting for the plugin's handlers: disconnected, so everything after is a gap`, "backlog"));
    };
    const deliver = (e: Event) => {
      if (e.epoch !== epoch || e.seq <= last) return; // another server run, in the snapshot already, or seen
      last = e.seq;
      if (++waiting > max) return overflow();
      queue = queue
        .then(() => handlers.onEvent(e))
        .catch((error) => console.error(`plugin: onEvent failed: ${error instanceof Error ? error.stack : error}`))
        .finally(() => waiting--);
    };
    this.listeners.push((e) => {
      if (ready) deliver(e);
      else if (early.push(e) > max) overflow();
    });
    const snapshot = await this.request<Snapshot>("events.subscribe", { snapshot: true, output: !!options.output });
    epoch = snapshot.epoch;
    last = snapshot.seq;
    await handlers.onSnapshot?.(snapshot);
    for (const e of early.sort((a, b) => a.seq - b.seq)) deliver(e);
    ready = true;
    return snapshot;
  }

  private send(message: object) {
    if (this.cause) return;
    try {
      this.transport.write(JSON.stringify(message) + "\n");
    } catch (error) {
      this.transport.close();
      this.drop(error instanceof Error ? error : new Error(String(error)));
    }
  }

  private async answer(id: number, { action, params, invocation, target, link, ui }: { action?: string; params?: Record<string, unknown>; invocation?: string; target?: { pane: string; instance: string }; link?: string; ui?: ViewEvent }) {
    const reply = (x: object) => this.send({ jsonrpc: "2.0", id, ...x });
    const run = action ? this.actions[action] : undefined;
    if (!run) return reply({ error: { code: -32601, message: `no action ${action}` } });
    const controller = new AbortController();
    if (invocation) this.running.set(invocation, controller);
    try {
      reply({ result: (await run(params ?? {}, { invocation, signal: controller.signal, ...(target && { target }), ...(link && { link }), ...(ui && { ui }) })) ?? null });
    } catch (error) {
      reply({ error: { code: -32000, message: error instanceof Error ? error.message : String(error) } });
    } finally {
      if (invocation) this.running.delete(invocation);
    }
  }
}

/**
 * Bytes waiting for the socket, in order. Bun's socket.write takes what fits (possibly nothing) and returns how many
 * bytes; the rest is sent as the socket drains. Past `limit` queued bytes, push throws: the session isn't reading.
 */
export function writeQueue(write: (bytes: Uint8Array) => number, limit = 64 * 1024 * 1024) {
  const queue: Uint8Array[] = [];
  const encoder = new TextEncoder();
  let queued = 0;
  const flush = () => {
    while (queue.length) {
      const chunk = queue[0]!;
      const n = Math.max(0, write(chunk));
      queued -= n;
      if (n < chunk.length) {
        queue[0] = chunk.subarray(n);
        return;
      }
      queue.shift();
    }
  };
  return {
    push(line: string) {
      const bytes = encoder.encode(line);
      if (queued + bytes.length > limit) throw new Error(`more than ${limit} bytes waiting to be sent: the session isn't reading`);
      queue.push(bytes);
      queued += bytes.length;
      flush();
    },
    flush,
    clear() {
      queue.length = 0;
      queued = 0;
    },
    get queued() {
      return queued;
    },
  };
}

type Socket = { write(data: Uint8Array): number; end(): void };
type Connector = (options: { unix: string; socket: Record<string, (...args: any[]) => void> }) => Promise<Socket>;

/** Connect to the session this plugin was started for ($MODISA_SOCKET). (`open` is for tests: a fake socket.) */
export async function connect(socket = Bun.env.MODISA_SOCKET, open: Connector = Bun.connect as unknown as Connector): Promise<Client> {
  if (!socket) throw new ModisaError("no $MODISA_SOCKET: modisa starts plugins with it set", "usage");
  let sock: Socket | undefined;
  let ended = false;
  const out = writeQueue((bytes) => sock?.write(bytes) ?? 0);
  // buffers cleared and the socket ended, once, however the connection goes
  const shut = () => {
    out.clear();
    if (ended || !sock) return;
    ended = true;
    sock.end();
  };
  const client = new Client({ write: (line) => out.push(line), close: shut });
  const gone = (error?: Error) => {
    shut();
    client.drop(error);
  };
  const decoder = new TextDecoder();
  sock = await open({
    unix: socket,
    socket: {
      // a write that fails while draining ends the connection like one that fails when sent
      drain: () => {
        try {
          out.flush();
        } catch (error) {
          gone(error instanceof Error ? error : new Error(String(error)));
        }
      },
      data: (_s, d) => client.feed(decoder.decode(d, { stream: true })),
      end: () => gone(),
      close: () => gone(),
      error: (_s, error) => gone(error),
    },
  });
  return client;
}

/** Run a plugin: connect, run `main`, and exit once the session's connection closes (the next server starts it again). */
export async function runPlugin(main: (modisa: Client) => unknown) {
  try {
    const modisa = await connect();
    await main(modisa);
    const why = await modisa.closed;
    console.log(`session connection closed (${why.message}); exiting`);
    process.exit(0);
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}

/**
 * For a plugin's own tests, run by `modisa plugin check`: the throwaway session it started with this plugin running.
 * `modisa(...args)` runs the CLI against it, `json(...args)` parses a --json reply, `data` is the plugin's data
 * directory, `until` waits for a condition.
 */
export function checkSession() {
  const raw = Bun.env.MODISA_CHECK;
  if (!raw) throw new Error("run these tests with `modisa plugin check <dir>`: it starts a throwaway session with this plugin running");
  const { bin, session, env, plugin, data } = JSON.parse(raw) as { bin: string[]; session: string; env: Record<string, string>; plugin: string; data: string };
  const modisa = async (...args: string[]) => {
    const p = Bun.spawn([...bin, "-s", session, ...args], { env: { ...Bun.env, ...env, MODISA_CHECK: "" }, stdout: "pipe", stderr: "pipe" });
    const [stdout, stderr] = await Promise.all([new Response(p.stdout).text(), new Response(p.stderr).text()]);
    return { code: await p.exited, stdout: stdout.trim(), stderr: stderr.trim() };
  };
  const json = async <T = any>(...args: string[]): Promise<T> => {
    const r = await modisa(...args, "--json");
    if (r.code !== 0) throw new Error(`modisa ${args.join(" ")} exited ${r.code}: ${r.stderr}`);
    return JSON.parse(r.stdout);
  };
  const until = async (what: string, ok: () => unknown, ms = 10_000) => {
    for (const end = Date.now() + ms; Date.now() < end; await Bun.sleep(100)) if (await ok()) return;
    throw new Error(`timed out waiting for ${what}`);
  };
  /** What the plugin shows in the TUI now (status, sidebar, badges, menu, palette actions). */
  const ui = () => json<UiState>("plugin", "ui", plugin);
  return { plugin, session, data, modisa, json, until, ui };
}
