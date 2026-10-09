// Charts a plugin view draws from its data, with ratatui's widgets: gauges and line gauges, sparklines, bar charts,
// charts (lines, scatter, bars, areas), canvases (shapes, the world map, text, in layers) and month calendars.
use ratatui::buffer::Buffer;
use ratatui::layout::{Direction, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols;
use ratatui::text::Line;
use ratatui::widgets::calendar::{CalendarEventStore, Monthly};
use ratatui::widgets::canvas::{self, Circle, Map, MapResolution, Points, Rectangle};
use ratatui::widgets::{Axis, Bar, BarChart, BarGroup, Chart, Dataset, Gauge, GraphType, LegendPosition, LineGauge, RenderDirection, Sparkline, SparklineBar, Widget};
use serde_json::Value;

use super::build::{ty, Ctx};
use crate::protocol::ui;

pub fn draw(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    match ty(n) {
        "gauge" => gauge(ctx, buf, n, area),
        "line_gauge" => line_gauge(ctx, buf, n, area),
        "sparkline" => sparkline(ctx, buf, n, area),
        "bar_chart" => bar_chart(ctx, buf, n, area),
        "chart" => chart(ctx, buf, n, area),
        "canvas" => canvas(ctx, buf, n, area),
        "calendar" => calendar(ctx, buf, n, area),
        _ => {}
    }
}

// 0 to 1, from `ratio` or `percent`
fn ratio(n: &Value) -> f64 {
    let r = n["ratio"].as_f64().or_else(|| n["percent"].as_f64().map(|p| p / 100.0)).unwrap_or(0.0);
    if r.is_finite() { r.clamp(0.0, 1.0) } else { 0.0 }
}

// a count for a widget that draws whole numbers: rounded, and nothing below 0
fn whole(v: &Value) -> u64 {
    v.as_f64().filter(|f| f.is_finite()).map_or(0, |f| f.max(0.0).round() as u64)
}

// a number as a label: no decimals when it has none
fn label(x: f64) -> String {
    if x.fract() == 0.0 && x.abs() < 1e15 { format!("{x:.0}") } else { format!("{x:.2}").trim_end_matches('0').to_string() }
}

fn gauge(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let mut g = Gauge::default()
        .ratio(ratio(n))
        .use_unicode(n["unicode"].as_bool().unwrap_or(true))
        .gauge_style(ctx.style_or(n, "gauge_style", Style::new().fg(ctx.tone("accent")).bg(ctx.tone("bar"))));
    if !n["label"].is_null() {
        g = g.label(ctx.span(&n["label"]));
    }
    g.render(area, buf);
}

fn line_gauge(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let mut g = LineGauge::default()
        .ratio(ratio(n))
        .filled_style(ctx.style_or(n, "filled_style", Style::new().fg(ctx.tone("accent"))))
        .unfilled_style(ctx.style_or(n, "unfilled_style", Style::new().fg(ctx.tone("border"))));
    if let Some(s) = n["filled_symbol"].as_str() {
        g = g.filled_symbol(s);
    }
    if let Some(s) = n["unfilled_symbol"].as_str() {
        g = g.unfilled_symbol(s);
    }
    if !n["label"].is_null() {
        g = g.label(Line::from(ctx.span(&n["label"])));
    }
    g.render(area, buf);
}

fn sparkline(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let data: Vec<SparklineBar> = n["data"].as_array().into_iter().flatten().map(|v| SparklineBar::from((!v.is_null()).then(|| whole(v)))).collect();
    let mut s = Sparkline::default()
        .data(data)
        .style(ctx.style_or(n, "style", Style::new().fg(ctx.tone("accent"))))
        .bar_set(if n["bar_set"] == "three_levels" { symbols::bar::THREE_LEVELS } else { symbols::bar::NINE_LEVELS })
        .direction(if n["direction"] == "right_to_left" { RenderDirection::RightToLeft } else { RenderDirection::LeftToRight })
        .absent_value_style(ctx.style_or(n, "absent_style", Style::new().fg(ctx.tone("dim"))));
    if let Some(sym) = n["absent_symbol"].as_str() {
        s = s.absent_value_symbol(sym);
    }
    if !n["max"].is_null() {
        s = s.max(whole(&n["max"]));
    }
    s.render(area, buf);
}

// A bar: `{ "value": n, "label": Line, "text_value": "…", "style": Style, "value_style": Style }`.
fn bar(ctx: &Ctx, b: &Value) -> Bar<'static> {
    let mut bar = Bar::default().value(whole(&b["value"]));
    if !b["label"].is_null() {
        bar = bar.label(ctx.line(&b["label"]));
    }
    if let Some(t) = b["text_value"].as_str() {
        bar = bar.text_value(t.to_string());
    }
    if !b["style"].is_null() {
        bar = bar.style(ctx.style(&b["style"]));
    }
    if !b["value_style"].is_null() {
        bar = bar.value_style(ctx.style(&b["value_style"]));
    }
    bar
}

fn bar_chart(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let groups: Vec<BarGroup> = match n["groups"].as_array() {
        Some(gs) => gs
            .iter()
            .map(|g| {
                let bars: Vec<Bar> = g["bars"].as_array().into_iter().flatten().map(|b| bar(ctx, b)).collect();
                let group = BarGroup::new(bars);
                if g["label"].is_null() { group } else { group.label(ctx.line(&g["label"])) }
            })
            .collect(),
        // `data`: one group of [label, value] pairs
        None => vec![BarGroup::new(
            n["data"].as_array().into_iter().flatten().map(|p| Bar::default().label(Line::from(p[0].as_str().unwrap_or("").to_string())).value(whole(&p[1]))).collect::<Vec<_>>(),
        )],
    };
    let small = |k: &str, d: u16| n[k].as_u64().map_or(d, |v| v.min(u16::MAX as u64) as u16);
    let mut c = BarChart::default()
        .direction(if n["direction"] == "horizontal" { Direction::Horizontal } else { Direction::Vertical })
        .bar_width(small("bar_width", 3))
        .bar_gap(small("bar_gap", 1))
        .group_gap(small("group_gap", 2))
        .bar_style(ctx.style_or(n, "bar_style", Style::new().fg(ctx.tone("accent"))))
        .label_style(ctx.style_or(n, "label_style", Style::new().fg(ctx.tone("dim"))));
    if !n["value_style"].is_null() {
        c = c.value_style(ctx.style(&n["value_style"]));
    }
    if !n["max"].is_null() {
        c = c.max(whole(&n["max"]));
    }
    for g in groups {
        c = c.data(g);
    }
    c.render(area, buf);
}

// An axis: `{ "title": Line, "bounds": [min, max], "labels": [Span…], "labels_align": …, "style": Style }`; without
// bounds it spans the data, and without labels it's labelled at its ends.
fn axis(ctx: &Ctx, v: &Value, data: (f64, f64)) -> Axis<'static> {
    let given = v["bounds"].as_array().and_then(|b| Some((b.first()?.as_f64()?, b.get(1)?.as_f64()?)));
    let (lo, hi) = given.unwrap_or(data);
    let (lo, hi) = if lo < hi { (lo, hi) } else { (lo - 1.0, hi + 1.0) };
    let labels: Vec<Line> = match v["labels"].as_array() {
        Some(ls) => ls.iter().map(|l| Line::from(ctx.span(l))).collect(),
        None => vec![Line::raw(label(lo)), Line::raw(label(hi))],
    };
    let mut a = Axis::default().bounds([lo, hi]).labels(labels).style(ctx.style_or(v, "style", Style::new().fg(ctx.tone("dim"))));
    if !v["title"].is_null() {
        a = a.title(ctx.line(&v["title"]));
    }
    if let Ok(Some(al)) = ui::align(&v["labels_align"]) {
        a = a.labels_alignment(al);
    }
    a
}

// the colours datasets take in turn when they don't say
const SERIES: [&str; 6] = ["accent", "done", "warn", "working", "blocked", "focus"];

fn chart(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let sets = n["datasets"].as_array().map(Vec::as_slice).unwrap_or_default();
    let data: Vec<Vec<(f64, f64)>> = sets
        .iter()
        .map(|s| s["data"].as_array().into_iter().flatten().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect())
        .collect();
    let span = |f: fn(&(f64, f64)) -> f64| {
        let all = data.iter().flatten().map(f);
        all.fold(None, |m: Option<(f64, f64)>, x| Some(m.map_or((x, x), |(lo, hi)| (lo.min(x), hi.max(x))))).unwrap_or((0.0, 1.0))
    };
    let datasets: Vec<Dataset> = sets
        .iter()
        .zip(&data)
        .enumerate()
        .map(|(i, (s, points))| {
            let graph = match s["graph_type"].as_str() {
                Some("scatter") => GraphType::Scatter,
                Some("bar") => GraphType::Bar,
                Some("area") => GraphType::Area,
                _ => GraphType::Line,
            };
            let mut d = Dataset::default()
                .data(points)
                .graph_type(graph)
                .marker(ui::marker(&s["marker"]).unwrap_or(symbols::Marker::Braille))
                .style(ctx.style_or(s, "style", Style::new().fg(ctx.tone(SERIES[i % SERIES.len()]))));
            if !s["name"].is_null() {
                d = d.name(ctx.line(&s["name"]));
            }
            if let Some(y) = s["fill_to"].as_f64() {
                d = d.fill_to_y(y);
            }
            d
        })
        .collect();
    let legend = match n["legend"].as_str() {
        Some("none") => None,
        Some("top_left") => Some(LegendPosition::TopLeft),
        Some("top") => Some(LegendPosition::Top),
        Some("left") => Some(LegendPosition::Left),
        Some("right") => Some(LegendPosition::Right),
        Some("bottom") => Some(LegendPosition::Bottom),
        Some("bottom_left") => Some(LegendPosition::BottomLeft),
        Some("bottom_right") => Some(LegendPosition::BottomRight),
        _ => Some(LegendPosition::TopRight),
    };
    Chart::new(datasets).x_axis(axis(ctx, &n["x_axis"], span(|p| p.0))).y_axis(axis(ctx, &n["y_axis"], span(|p| p.1))).legend_position(legend).render(area, buf);
}

enum Shape {
    Line(canvas::Line),
    Rectangle(Rectangle),
    Circle(Circle),
    Points(Vec<(f64, f64)>, Color),
    Map(MapResolution, Color),
    Text(f64, f64, Line<'static>),
    Layer,
}

fn shape(ctx: &Ctx, s: &Value) -> Option<Shape> {
    let c = ctx.color(&s["color"]).unwrap_or(ctx.tone("fg"));
    let nums = |k: &str| -> Option<Vec<f64>> { s[k].as_array()?.iter().map(Value::as_f64).collect() };
    Some(if s["layer"] == true {
        Shape::Layer
    } else if let Some([x1, y1, x2, y2]) = nums("line").as_deref() {
        Shape::Line(canvas::Line::new(*x1, *y1, *x2, *y2, c))
    } else if let Some([x, y, w, h]) = nums("rectangle").as_deref() {
        Shape::Rectangle(Rectangle::new(*x, *y, *w, *h, c))
    } else if let Some([x, y, r]) = nums("circle").as_deref() {
        Shape::Circle(Circle::new(*x, *y, *r, c))
    } else if let Some(ps) = s["points"].as_array() {
        Shape::Points(ps.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect(), c)
    } else if let Some(m) = s["map"].as_str() {
        Shape::Map(if m == "high" { MapResolution::High } else { MapResolution::Low }, c)
    } else if !s["text"].is_null() {
        let at = nums("at").unwrap_or_default();
        Shape::Text(*at.first()?, *at.get(1)?, ctx.line(&s["text"]))
    } else {
        return None;
    })
}

fn canvas(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    let shapes: Vec<Shape> = n["shapes"].as_array().into_iter().flatten().filter_map(|s| shape(ctx, s)).collect();
    // without bounds: the world for a map, else 0 to 100 each way
    let world = shapes.iter().any(|s| matches!(s, Shape::Map(..)));
    let bounds = |k: &str, d: [f64; 2]| n[k].as_array().and_then(|b| Some([b.first()?.as_f64()?, b.get(1)?.as_f64()?])).unwrap_or(d);
    let mut c = canvas::Canvas::default()
        .x_bounds(bounds("x_bounds", if world { [-180.0, 180.0] } else { [0.0, 100.0] }))
        .y_bounds(bounds("y_bounds", if world { [-90.0, 90.0] } else { [0.0, 100.0] }))
        .marker(ui::marker(&n["marker"]).unwrap_or(symbols::Marker::Braille))
        .paint(|p| {
            for s in &shapes {
                match s {
                    Shape::Line(l) => p.draw(l),
                    Shape::Rectangle(r) => p.draw(r),
                    Shape::Circle(c) => p.draw(c),
                    Shape::Points(ps, c) => p.draw(&Points::new(ps, *c)),
                    Shape::Map(r, c) => p.draw(&Map { resolution: *r, color: *c }),
                    Shape::Text(x, y, l) => p.print(*x, *y, l.clone()),
                    Shape::Layer => p.layer(),
                }
            }
        });
    if let Some(bg) = ctx.color(&n["background"]) {
        c = c.background_color(bg);
    }
    c.render(area, buf);
}

// A month: its header and the weekdays' unless `false`, other months' days only in a style given for them, and the
// days in `events` in theirs.
fn calendar(ctx: &Ctx, buf: &mut Buffer, n: &Value, area: Rect) {
    use time::{Date, Month, OffsetDateTime};
    let today = OffsetDateTime::now_utc().date();
    let year = n["year"].as_i64().map_or(today.year(), |y| y.clamp(-9999, 9999) as i32);
    let month = n["month"].as_u64().and_then(|m| Month::try_from(m as u8).ok()).unwrap_or(today.month());
    let Ok(first) = Date::from_calendar_date(year, month, 1) else { return };
    let mut events = CalendarEventStore::default();
    for (day, style) in n["events"].as_object().into_iter().flatten() {
        let mut parts = day.splitn(3, '-').map(|p| p.parse::<i32>().ok());
        if let (Some(Some(y)), Some(Some(m)), Some(Some(d))) = (parts.next(), parts.next(), parts.next()) {
            if let Ok(date) = Month::try_from(m as u8).and_then(|m| Date::from_calendar_date(y, m, d as u8)) {
                events.add(date, ctx.style(style));
            }
        }
    }
    let mut cal = Monthly::new(first, events).default_style(ctx.style_or(n, "default_style", Style::new().fg(ctx.tone("fg"))));
    if n["month_header"] != false {
        cal = cal.show_month_header(ctx.style_or(n, "month_header", Style::new().fg(ctx.tone("accent")).add_modifier(Modifier::BOLD)));
    }
    if n["weekday_header"] != false {
        cal = cal.show_weekdays_header(ctx.style_or(n, "weekday_header", Style::new().fg(ctx.tone("dim"))));
    }
    if !n["surrounding"].is_null() && n["surrounding"] != false {
        cal = cal.show_surrounding(ctx.style(&n["surrounding"]));
    }
    cal.render(area, buf);
}
