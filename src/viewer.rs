//! A read-only code view drawn as a single widget.

use crate::find::FindMatch;
use crate::highlight::{Document, Rgba};
use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer::{self, Quad};
use iced::advanced::text::{self, Paragraph as _, Renderer as _, Span, Text};
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Renderer as _, Shell, mouse};
use iced::alignment::Vertical;
use iced::font::{Style as FontStyle, Weight};
use iced::{Border, Color, Element, Event, Font, Length, Pixels, Point, Rectangle, Shadow, Size};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Instant;
use unicode_segmentation::UnicodeSegmentation;

type P = <iced::Renderer as text::Renderer>::Paragraph;

pub const LINE_HEIGHT: f32 = 19.0;
const FONT: Font = Font::with_name("Consolas");
const FONT_SIZE: f32 = 14.0;
const GUTTER_PAD: f32 = 24.0;
/// Longest line prefix handed to the text shaper; protects against minified files.
const MAX_SHAPED_CHARS: usize = 4000;
/// Shaped lines kept either side of the visible range.
const KEEP_AROUND: usize = 300;

pub static RECORD: AtomicBool = AtomicBool::new(false);
pub static DRAW_MICROS: Mutex<Vec<f32>> = Mutex::new(Vec::new());
pub static SHAPED_LINES: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy)]
pub struct Palette {
    pub foreground: Color,
    pub line_number: Color,
}

pub fn color(c: Rgba) -> Color {
    Color::from_rgba(c.0, c.1, c.2, c.3)
}

pub struct CodeView<'a, M> {
    doc: &'a Document,
    generation: u64,
    palette: Palette,
    selected: Option<u32>,
    ignored: &'a HashSet<u64>,
    on_click: Box<dyn Fn(u32) -> M + 'a>,
    /// Published when a click lands on anything that isn't a comment.
    on_miss: M,
    /// Find results for this document, in order.
    matches: &'a [FindMatch],
    current_match: Option<usize>,
}

impl<'a, M> CodeView<'a, M> {
    pub fn new(
        doc: &'a Document,
        generation: u64,
        palette: Palette,
        selected: Option<u32>,
        ignored: &'a HashSet<u64>,
        on_click: impl Fn(u32) -> M + 'a,
        on_miss: M,
    ) -> Self {
        Self {
            doc,
            generation,
            palette,
            selected,
            ignored,
            on_click: Box::new(on_click),
            on_miss,
            matches: &[],
            current_match: None,
        }
    }

    pub fn find_results(mut self, matches: &'a [FindMatch], current: Option<usize>) -> Self {
        self.matches = matches;
        self.current_match = current;
        self
    }

    /// Where each of a line's find results is drawn, relative to the start of the code.
    fn match_spans(&self, line: usize, shaped: &Shaped) -> Vec<(f32, f32, bool)> {
        let from = self.matches.partition_point(|m| (m.line as usize) < line);
        let on_line = self.matches[from..].iter().take_while(|m| m.line as usize == line);
        let mut spans = Vec::new();
        let mut text = None;
        for (offset, found) in on_line.enumerate() {
            let text = text.get_or_insert_with(|| {
                self.doc.lines[line].segments.iter().map(|s| s.text.as_str()).collect::<String>()
            });
            let (Some(before), Some(through)) =
                (text.get(..found.start as usize), text.get(..found.end as usize))
            else {
                continue;
            };
            let start = before.graphemes(true).count();
            let end = through.graphemes(true).count();
            if start >= MAX_SHAPED_CHARS {
                continue;
            }
            let (Some(left), Some(right)) = (
                shaped.code.grapheme_position(0, start),
                shaped.code.grapheme_position(0, end.min(MAX_SHAPED_CHARS)),
            ) else {
                continue;
            };
            spans.push((left.x, right.x, self.current_match == Some(from + offset)));
        }
        spans
    }

    fn gutter_width(&self, advance: f32) -> f32 {
        let digits = self.doc.lines.len().max(1).ilog10() as f32 + 1.0;
        digits * advance + GUTTER_PAD
    }

    /// A band across the whole row, rather than a box around the comment text.
    fn is_row_band(&self, line: usize) -> bool {
        !self.doc.lines[line].has_code() && self.doc.line_hunks[line].len() == 1
    }

    fn shape(&self, index: usize) -> Shaped {
        SHAPED_LINES.fetch_add(1, Ordering::Relaxed);

        let hunk_ids = &self.doc.line_hunks[index];
        let mut budget = MAX_SHAPED_CHARS;
        let mut spans: Vec<Span<'_, (), Font>> = Vec::new();
        let mut comment_spans: Vec<(u32, usize)> = Vec::new();
        let mut column = 0;

        for segment in &self.doc.lines[index].segments {
            if budget == 0 {
                break;
            }
            let chars = segment.text.chars().count();
            let content = if chars <= budget {
                budget -= chars;
                segment.text.as_str()
            } else {
                let cut = segment
                    .text
                    .char_indices()
                    .nth(budget)
                    .map_or(segment.text.len(), |(i, _)| i);
                budget = 0;
                &segment.text[..cut]
            };
            if segment.comment && !content.trim().is_empty() {
                let leading = content.len() - content.trim_start().len();
                let owner = hunk_ids.iter().copied().find(|&id| {
                    let hunk = &self.doc.hunks[id as usize];
                    let from = if hunk.first == index { hunk.start } else { 0 };
                    let to = if hunk.last == index { hunk.end } else { usize::MAX };
                    (from..to).contains(&(column + leading))
                });
                if let Some(id) = owner {
                    comment_spans.push((id, spans.len()));
                }
            }
            column += segment.text.len();
            let font = Font {
                weight: if segment.bold { Weight::Bold } else { Weight::Normal },
                style: if segment.italic { FontStyle::Italic } else { FontStyle::Normal },
                ..FONT
            };
            spans.push(Span::new(content).color(color(segment.color)).font(font));
        }

        let code = P::with_spans(base_text(spans.as_slice()));
        let mut comments: Vec<(u32, Rectangle)> = Vec::new();
        for (id, span) in comment_spans {
            for rect in code.span_bounds(span) {
                match comments.iter_mut().find(|(existing, _)| *existing == id) {
                    Some((_, area)) => *area = area.union(&rect),
                    None => comments.push((id, rect)),
                }
            }
        }
        let number = (index + 1).to_string();
        let gutter = P::with_text(base_text(number.as_str()));

        Shaped {
            code,
            gutter,
            comments,
        }
    }

    /// The hunk under the cursor. Row bands hit anywhere on the row; a comment sharing its line with code hits only over its own text.
    fn hit(
        &self,
        cursor: mouse::Cursor,
        bounds: Rectangle,
        viewport: &Rectangle,
        advance: f32,
        cache: &HashMap<usize, Shaped>,
    ) -> Option<u32> {
        let point = cursor.position().filter(|p| viewport.contains(*p))?;
        if point.y < bounds.y {
            return None;
        }
        let line = ((point.y - bounds.y) / LINE_HEIGHT) as usize;
        let ids = self.doc.line_hunks.get(line)?;
        if ids.is_empty() {
            return None;
        }
        if self.is_row_band(line) {
            return Some(ids[0]);
        }
        let x = point.x - (bounds.x + self.gutter_width(advance));
        cache
            .get(&line)?
            .comments
            .iter()
            .find(|(_, rect)| rect.x <= x && x <= rect.x + rect.width)
            .map(|(id, _)| *id)
    }
}

struct Shaped {
    code: P,
    gutter: P,
    /// Where each comment sits, relative to the start of the code.
    comments: Vec<(u32, Rectangle)>,
}

#[derive(Default)]
struct State {
    advance: Cell<f32>,
    generation: Cell<u64>,
    hovered: Cell<Option<u32>>,
    cache: RefCell<HashMap<usize, Shaped>>,
}

fn base_text<C>(content: C) -> Text<C, Font> {
    Text {
        content,
        bounds: Size::INFINITE,
        size: Pixels(FONT_SIZE),
        line_height: text::LineHeight::Absolute(Pixels(LINE_HEIGHT)),
        font: FONT,
        align_x: text::Alignment::Default,
        align_y: Vertical::Top,
        shaping: text::Shaping::Auto,
        wrapping: text::Wrapping::None,
    }
}

impl<M: Clone, Th> Widget<M, Th, iced::Renderer> for CodeView<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Shrink, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        _limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_ref::<State>();
        if state.advance.get() == 0.0 {
            let sample = P::with_text(base_text("M"));
            state.advance.set(sample.min_bounds().width);
        }
        let advance = state.advance.get();
        let chars = self.doc.max_chars.min(MAX_SHAPED_CHARS) as f32;
        layout::Node::new(Size::new(
            self.gutter_width(advance) + chars * advance + GUTTER_PAD,
            self.doc.lines.len() as f32 * LINE_HEIGHT,
        ))
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<State>();
        let hit = || {
            self.hit(cursor, layout.bounds(), viewport, state.advance.get(), &state.cache.borrow())
        };
        match event {
            Event::Mouse(mouse::Event::CursorMoved { .. } | mouse::Event::CursorLeft) => {
                if hit() != state.hovered.get() {
                    shell.request_redraw();
                }
            }
            Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                if let Some(id) = hit() {
                    shell.publish((self.on_click)(id));
                    shell.capture_event();
                } else if cursor.position().is_some_and(|p| viewport.contains(p)) {
                    shell.publish(self.on_miss.clone());
                }
            }
            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        match tree.state.downcast_ref::<State>().hovered.get() {
            Some(_) => mouse::Interaction::Pointer,
            None => mouse::Interaction::default(),
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        _theme: &Th,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let started = Instant::now();
        let state = tree.state.downcast_ref::<State>();
        let bounds = layout.bounds();
        let mut cache = state.cache.borrow_mut();

        if state.generation.replace(self.generation) != self.generation {
            cache.clear();
        }

        let total = self.doc.lines.len();
        let first = (((viewport.y - bounds.y) / LINE_HEIGHT).floor().max(0.0) as usize).min(total);
        let last = (((viewport.y + viewport.height - bounds.y) / LINE_HEIGHT).ceil() as usize).min(total);

        if cache.len() > KEEP_AROUND * 4 {
            let keep = first.saturating_sub(KEEP_AROUND)..last + KEEP_AROUND;
            cache.retain(|line, _| keep.contains(line));
        }
        for index in first..last {
            cache.entry(index).or_insert_with(|| self.shape(index));
        }

        let advance = state.advance.get();
        let hovered = self.hit(cursor, bounds, viewport, advance, &cache);
        state.hovered.set(hovered);

        let gutter = self.gutter_width(advance);
        let code_x = bounds.x + gutter;
        let code_clip = Rectangle {
            x: viewport.x + gutter,
            width: (viewport.width - gutter).max(0.0),
            ..*viewport
        };
        let quad = |bounds: Rectangle| Quad {
            bounds,
            border: Border::default(),
            shadow: Shadow::default(),
            snap: true,
        };
        let band = |id: u32| {
            let dimmed = self.ignored.contains(&self.doc.hunks[id as usize].key);
            let alpha = if self.selected == Some(id) {
                0.34
            } else if hovered == Some(id) {
                0.22
            } else if dimmed {
                0.03
            } else {
                0.09
            };
            if dimmed && self.selected != Some(id) {
                Color::from_rgba(0.7, 0.7, 0.7, alpha)
            } else {
                Color::from_rgba(1.0, 0.8, 0.2, alpha)
            }
        };

        for index in first..last {
            let shaped = &cache[&index];
            let y = bounds.y + index as f32 * LINE_HEIGHT;

            if self.is_row_band(index) && !self.doc.line_hunks[index].is_empty() {
                let id = self.doc.line_hunks[index][0];
                let row = Rectangle {
                    x: viewport.x,
                    y,
                    width: viewport.width,
                    height: LINE_HEIGHT,
                };
                renderer.fill_quad(quad(row), band(id));
                if !self.ignored.contains(&self.doc.hunks[id as usize].key) {
                    let bar = Rectangle { width: 3.0, ..row };
                    renderer.fill_quad(quad(bar), Color::from_rgb(1.0, 0.75, 0.2));
                }
            } else {
                for (id, rect) in &shaped.comments {
                    let area = Rectangle {
                        x: code_x + rect.x,
                        y,
                        width: rect.width,
                        height: LINE_HEIGHT,
                    };
                    if let Some(area) = area.intersection(&code_clip) {
                        renderer.fill_quad(quad(area), band(*id));
                    }
                }
            }

            for (left, right, current) in self.match_spans(index, shaped) {
                let area = Rectangle {
                    x: code_x + left,
                    y,
                    width: (right - left).max(2.0),
                    height: LINE_HEIGHT,
                };
                if let Some(area) = area.intersection(&code_clip) {
                    // Filled lightly and outlined, so the text stays readable on top.
                    let (tint, edge) = if current {
                        (Color::from_rgba(1.0, 0.55, 0.05, 0.34), 1.5)
                    } else {
                        (Color::from_rgba(0.2, 0.65, 1.0, 0.30), 0.0)
                    };
                    let border = Border { color: Color::from_rgb(1.0, 0.6, 0.1), width: edge, radius: 2.0.into() };
                    renderer.fill_quad(Quad { border, ..quad(area) }, tint);
                }
            }

            renderer.fill_paragraph(
                &shaped.gutter,
                Point::new(viewport.x + 10.0, y),
                self.palette.line_number,
                *viewport,
            );
            renderer.fill_paragraph(
                &shaped.code,
                Point::new(code_x, y),
                self.palette.foreground,
                code_clip,
            );
        }

        if RECORD.load(Ordering::Relaxed) {
            DRAW_MICROS
                .lock()
                .unwrap()
                .push(started.elapsed().as_secs_f32() * 1e6);
        }
    }
}

impl<'a, M: Clone + 'a, Th: 'a> From<CodeView<'a, M>> for Element<'a, M, Th, iced::Renderer> {
    fn from(view: CodeView<'a, M>) -> Self {
        Element::new(view)
    }
}
