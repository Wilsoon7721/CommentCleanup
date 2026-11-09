//! Application state, messages and layout

use crate::ai::{self, AiSettings, Preset, Provider, Sent, Suggested};
use crate::cleanup::{self, Applied, Change};
use crate::files::{self, Entry, Loaded};
use crate::find::{self, FindMatch, Query, SearchFile};
use crate::git::{self, Changed, FileState, Repo};
use crate::highlight::{Document, Highlighter, Hunk, HunkKind, parse_hex};
use crate::palette;
use crate::rules::{self, Decision, Rules, Settings};
use crate::scan::{self, FolderUndo, ImpactFile, Plan, PlanFile, PlanFileChanges, PlanItem, PlanOutcome, RuleImpact, ScannedFile, Tally};
use crate::session::{self, Session};
use crate::update::{self, NoteLine, Release};
use crate::icons::{self, Icon, IconTheme, LoadedIcons, glyph};
use crate::viewer::{self, CodeView, LINE_HEIGHT, Palette, color};
use crate::vscode::{self, IconThemeEntry};
use crate::window_state::{self, Placement};
use iced::advanced::text::Span;
use iced::event::{self, Event};
use iced::font::Weight;
use iced::futures::channel::{mpsc, oneshot};
use iced::futures::{SinkExt, Stream, StreamExt};
use iced::keyboard::{self, Key, Modifiers, key::Named};
use iced::task;
use iced::widget::scrollable::{AbsoluteOffset, Direction, Scrollbar, Viewport};
use iced::widget::{
    Id, button, center, checkbox, column, container, mouse_area, opaque, operation, pick_list,
    responsive, rich_text, row, scrollable, space, stack, svg, text, text_editor, text_input, tooltip,
};
use iced::window::Direction as Edge;
use iced::{
    Background, Border, Color, Element, Fill, Font, Length, Padding, Shadow, Size, Subscription,
    Radians, Rotation, Task, Theme, Vector, mouse, window,
};
use notify::{EventKind, RecursiveMode, Watcher};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const CODE_SCROLL: &str = "code";
const SIDEBAR_WIDTH: f32 = 260.0;
/// Lines the sidebar's footer up with the status bar beside it.
const SIDEBAR_FOOTER_ROW: f32 = 18.0;
const AUTOSCROLL_FRAMES: usize = 1000;
const MAX_UNDO: usize = 50;
const MAX_FOLDER_UNDO: usize = 5;
/// Beyond this many changed paths at once, rescanning the folder is simpler.
const MAX_INCREMENTAL_RESCAN: usize = 200;
/// Longest time the sidebar's counts may lag during a scan.
const RECOUNT_INTERVAL: Duration = Duration::from_millis(250);
/// Changes listed per file in the preview. The rest still apply.
const MAX_PREVIEW_ITEMS: usize = 300;
/// Lines shown above a hunk when jumping to it.
const CONTEXT_LINES: usize = 5;
const REWORD_EDITOR: &str = "reword";
const MENU_BAR_HEIGHT: f32 = 32.0;
const WINDOW_BUTTON_WIDTH: f32 = 46.0;
const APP_ICON_SLOT: f32 = 32.0;
const APP_ICON: &[u8] = include_bytes!("../assets/icon-small.svg");
/// Keeps a long title clear of the menus and window buttons, staying centred.
const TITLE_SIDE_ROOM: f32 = 160.0;
const RESIZE_EDGE: f32 = 5.0;
const RESIZE_CORNER: f32 = 12.0;
/// Two presses on the title bar within this long count as a double-click.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const MENU_WIDTH: f32 = 250.0;
const ICON_SIZE: f32 = 16.0;
const TOOLBAR_HEIGHT: f32 = 34.0;
const TOOLBAR_GAP: f32 = 3.0;
const REWORD_HEIGHT: f32 = 156.0;
const REWORD_WIDTH: f32 = 760.0;
/// Space kept clear of the code view's scrollbars.
const SCROLLBAR_ROOM: f32 = 14.0;
/// Below this width the toolbar drops its description to keep the buttons.
const WIDE_TOOLBAR: f32 = 960.0;
const SPARKLE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M10.5 3l2.3 7.2L20 12.5l-7.2 2.3L10.5 22l-2.3-7.2L1 12.5l7.2-2.3z"/><path d="M19 1l1 2.9L23 5l-3 1-1 3-1-3-3-1 3-1.1z"/><path d="M19.5 16l.7 2.1 2.1.7-2.1.7-.7 2.1-.7-2.1-2.1-.7 2.1-.7z"/></svg>"#;
/// Rows of a suggestion shown before it scrolls, and rough wrapping for placing it.
const SUGGESTION_ROWS: usize = 12;
const SUGGESTION_ROW_CHARS: usize = 100;
const SUGGESTION_ROW_HEIGHT: f32 = 16.0;
const FIND_INPUT: &str = "find";
const SEARCH_INPUT: &str = "search";
const SEARCH_SCROLL: &str = "search-results";
const NO_WIDGET: &str = "no-widget";
const PALETTE_INPUT: &str = "palette";
const PALETTE_WIDTH: f32 = 560.0;
const PALETTE_ROWS: usize = 12;
/// How far ahead suggestions are fetched while going through all.
const AHEAD_COMMENTS: usize = 3;
const AHEAD_AT_ONCE: usize = 2;
const AHEAD_PAUSE: Duration = Duration::from_secs(30);
const SUGGEST_COOLDOWN: Duration = Duration::from_secs(30);
const SPINNER: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="3" stroke-linecap="round"><path d="M12 3a9 9 0 1 1-9 9"/></svg>"##;
/// How often a copy left open looks for a new release.
const UPDATE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);
const FIND_BAR_HEIGHT: f32 = 36.0;
const FIND_BAR_WIDTH: f32 = 460.0;
/// Results are a fixed height, so only the ones on screen need building.
const SEARCH_ROW_HEIGHT: f32 = 22.0;
/// Roughly how wide a character of code is, for scrolling a result into view.
const CODE_ADVANCE: f32 = 7.7;
const CODE_GUTTER: f32 = 60.0;
/// A pause so typing in a pattern box doesn't re-test the folder per letter.
const IMPACT_DELAY: Duration = Duration::from_millis(120);
const BRANCH: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2"><circle cx="6" cy="5" r="2.5"/><circle cx="6" cy="19" r="2.5"/><circle cx="18" cy="7" r="2.5"/><path d="M6 7.5v9M18 9.5c0 4-12 2-12 7"/></svg>"##;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    File,
    Edit,
    View,
}

const MENUS: [(Menu, &str, f32); 3] = [
    (Menu::File, "File", 44.0),
    (Menu::Edit, "Edit", 44.0),
    (Menu::View, "View", 50.0),
];

/// Development switches; the app itself needs none of these.
#[derive(Clone, Default)]
pub struct Options {
    pub folder: Option<PathBuf>,
    pub open: Option<PathBuf>,
    pub theme: Option<String>,
    /// Pretend no editors are installed, to test the bundled themes and grammars.
    pub standalone: bool,
    pub autoscroll: bool,
    pub step: f32,
    pub shot: Option<PathBuf>,
    pub delay: u64,
    pub exit: bool,
}

struct Auto {
    y: f32,
    frames: usize,
    last: Option<Instant>,
    intervals: Vec<f32>,
}

struct Tab {
    path: PathBuf,
    name: String,
    doc: Arc<Document>,
    hash: u64,
    generation: u64,
    read_only: Option<String>,
    load_seq: u64,
    offset: AbsoluteOffset,
    selected: Option<usize>,
    /// Hunks marked Ignore this session, by hunk key.
    ignored: HashSet<u64>,
    /// What the rules say about each hunk.
    decisions: Vec<Decision>,
    /// Keys of ignored and protected hunks: drawn dim and skipped when going through.
    dimmed: HashSet<u64>,
    /// Earlier file contents, each with the hash of the state its edit produced, so undo can tell whether anything else has changed the file since.
    undo: Vec<(Arc<Vec<u8>>, u64)>,
    /// `Some(count reviewed)` while going through all comments.
    review: Option<usize>,
    /// Comments whose "Include function definition" differs from the default.
    function_override: HashMap<u64, bool>,
}

enum SuggestionState {
    Loading,
    Ready(Suggested),
    Failed(String),
}

/// What the button beside the dropdown does to the whole file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FixMode {
    /// Whatever the rules decide for each comment.
    #[default]
    Auto,
    Delete,
    Squash,
}

impl FixMode {
    const ALL: [FixMode; 3] = [FixMode::Auto, FixMode::Delete, FixMode::Squash];

    fn button(self, count: usize) -> String {
        match self {
            FixMode::Auto => format!("Fix all ({count})"),
            FixMode::Delete => format!("Delete all ({count})"),
            FixMode::Squash => format!("Squash all ({count})"),
        }
    }

    fn menu_label(self) -> &'static str {
        match self {
            FixMode::Auto => "Fix All in This File",
            FixMode::Delete => "Delete All in This File",
            FixMode::Squash => "Squash All in This File",
        }
    }

    fn title(self) -> &'static str {
        match self {
            FixMode::Auto => "Fix all in this file",
            FixMode::Delete => "Delete all comments in this file",
            FixMode::Squash => "Squash all comments in this file",
        }
    }

    fn note(self) -> &'static str {
        match self {
            FixMode::Auto => "Nothing changes until you apply. One Undo restores the file.",
            FixMode::Delete => "Every comment that isn't protected or ignored. Nothing changes until you apply. One Undo restores the file.",
            FixMode::Squash => "Every multi-line comment that isn't protected or ignored. Nothing changes until you apply. One Undo restores the file.",
        }
    }

    /// Whether this mode would change a comment.
    fn applies(self, hunk: &Hunk, decision: &Decision, syntax: Option<&vscode::CommentSyntax>) -> bool {
        match self {
            FixMode::Auto => decision.is_fix(),
            FixMode::Delete => !decision.is_protected(),
            FixMode::Squash => !decision.is_protected() && cleanup::squash(hunk, syntax).is_some(),
        }
    }

    fn item(self, hunk: &Hunk, decision: &Decision, syntax: Option<&vscode::CommentSyntax>) -> Option<PlanItem> {
        let (change, reason) = match self {
            FixMode::Auto => return plan_item(hunk, decision, true),
            _ if !self.applies(hunk, decision, syntax) => return None,
            FixMode::Delete => (Change::Delete, "Delete".to_owned()),
            FixMode::Squash => (Change::Replace(cleanup::squash(hunk, syntax)?), "Squash into one line".to_owned()),
        };
        Some(PlanItem { hunk: hunk.clone(), change, reason, include: true })
    }
}

impl std::fmt::Display for FixMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            FixMode::Auto => "Auto fix",
            FixMode::Delete => "Delete",
            FixMode::Squash => "Squash",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SidePanel {
    Files,
    Search,
}

#[derive(Debug, Clone, Copy)]
pub enum FindOption {
    Case,
    Word,
    Regex,
}

impl FindOption {
    fn flip(self, query: &mut Query) {
        match self {
            FindOption::Case => query.match_case = !query.match_case,
            FindOption::Word => query.whole_word = !query.whole_word,
            FindOption::Regex => query.regex = !query.regex,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SearchRow {
    File(usize),
    Hit(usize, usize),
}

enum MenuEntry {
    Item(&'static str, &'static str, Option<Message>),
    Separator,
}

enum AheadState {
    Loading,
    Ready(Suggested),
    Failed(String),
}

struct CachedSuggestion {
    asked: Instant,
    settings: (String, String, bool),
    reply: Option<Suggested>,
}

enum SuggestGate {
    Cached(Suggested),
    Wait(u64),
    Ask,
}

enum UpdateState {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading { version: String, percent: u64, notes: String },
    Ready { version: String, path: PathBuf, notes: String },
    Failed(String),
}

struct Suggestion {
    key: u64,
    state: SuggestionState,
    rejected: Vec<String>,
}

pub struct App {
    options: Options,
    highlighter: Option<Arc<Highlighter>>,
    theme_name: String,
    theme: Theme,
    palette: Palette,
    background: Color,
    root: Option<PathBuf>,
    expanded: HashSet<PathBuf>,
    entries: Vec<Entry>,
    tabs: Vec<Tab>,
    active: Option<usize>,
    reword: Option<text_editor::Content>,
    busy: bool,
    status: String,
    next_generation: u64,
    auto: Option<Auto>,
    settings: Settings,
    rules: Arc<Rules>,
    folder_settings: bool,
    show_settings: bool,
    plan: Option<Plan>,
    folder_undo: Vec<Vec<FolderUndo>>,
    scan: HashMap<PathBuf, ScannedFile>,
    scan_running: bool,
    scan_generation: u64,
    scan_abort: Option<task::Handle>,
    scan_stale: bool,
    last_recount: Instant,
    counts: HashMap<PathBuf, usize>,
    ignored_store: HashMap<PathBuf, HashSet<u64>>,
    restore: Option<Session>,
    pending_review: Option<PathBuf>,
    protect_patterns: text_editor::Content,
    delete_patterns: text_editor::Content,
    width_input: String,
    lines_input: String,
    menu: Option<Menu>,
    maximized: bool,
    last_title_press: Option<Instant>,
    title_armed: bool,
    ctrl_held: bool,
    show_sidebar: bool,
    icon_theme: Option<Arc<LoadedIcons>>,
    icon_theme_name: String,
    icon_theme_names: Vec<String>,
    entry_icons: Vec<Icon>,
    ai: AiSettings,
    ai_custom: text_editor::Content,
    ai_key_input: String,
    ai_has_key: bool,
    ai_models: Vec<String>,
    ai_models_error: Option<String>,
    suggestion: Option<Suggestion>,
    suggest_seq: u64,
    saved_settings: Settings,
    edited_rules: Arc<Rules>,
    rule_impact: Option<RuleImpact>,
    impact_seq: Arc<AtomicU64>,
    rule_sample: text_editor::Content,
    rule_sample_result: Option<String>,
    repo: Option<Repo>,
    repo_dirs: HashSet<PathBuf>,
    git_running: bool,
    git_again: bool,
    only_changed: bool,
    find_open: bool,
    find_query: Query,
    find_matches: Arc<Vec<FindMatch>>,
    find_current: Option<usize>,
    find_error: Option<String>,
    find_seq: u64,
    find_reveal: bool,
    find_goto: Option<(PathBuf, u32, u32)>,
    shift_held: bool,
    code_view: Size,
    side_panel: SidePanel,
    search_query: Query,
    search_files: Vec<SearchFile>,
    search_rows: Vec<SearchRow>,
    search_total: usize,
    search_running: bool,
    search_error: Option<String>,
    search_collapsed: HashSet<PathBuf>,
    search_seq: u64,
    search_abort: Option<task::Handle>,
    search_replacing: bool,
    search_scroll: (f32, f32),
    confirm_exit: Option<Vec<&'static str>>,
    placement: Option<Placement>,
    update: UpdateState,
    auto_update: bool,
    update_restart: bool,
    update_abort: Option<task::Handle>,
    update_notes: Option<(String, String)>,
    whats_new: Option<(String, Vec<NoteLine>)>,
    ahead: HashMap<u64, AheadState>,
    ahead_epoch: u64,
    ahead_paused: Option<Instant>,
    suggest_cache: HashMap<(PathBuf, u64), CachedSuggestion>,
    spinner_start: Instant,
    palette_open: bool,
    palette_query: String,
    palette_selected: usize,
    fix_mode: FixMode,
}

#[derive(Default, Clone, Copy)]
struct Available {
    fix: bool,
    ignore: bool,
    squash: bool,
    delete: bool,
    reword: bool,
    suggest: bool,
}

type BoolField = for<'a> fn(&'a mut Settings) -> &'a mut bool;

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Fix,
    Ignore,
    Squash,
    Delete,
    Reword,
    Suggest,
}

#[derive(Debug, Clone)]
pub enum Message {
    HighlighterReady(Result<Arc<Highlighter>, String>),
    PickFolder,
    FolderPicked(Option<PathBuf>),
    PickFile,
    FilePicked(Option<PathBuf>),
    CloseFile,
    CloseFolder,
    Exit,
    ExitConfirmed,
    ExitCancelled,
    Quit(u64),
    OpenMenu(Menu),
    HoverMenu(Menu),
    CloseMenu,
    MenuItem(Box<Message>),
    TitleBarPressed,
    TitleBarReleased,
    TitleBarDragged,
    WindowMinimize,
    WindowToggleMaximize,
    WindowResize(Edge),
    WindowResized,
    MaximizedChanged(bool),
    ToggleSidebar,
    KeyPressed(Key, Modifiers, bool),
    ModifiersChanged(Modifiers),
    CodeClicked,
    IconThemePicked(String),
    IconsLoaded(String, Result<Arc<LoadedIcons>, String>),
    IconFontReady(String, Arc<LoadedIcons>),
    EntryPressed(usize),
    ActivateTab(usize),
    CloseTab(usize),
    Loaded(PathBuf, u64, Result<Option<Loaded>, String>),
    FilesChanged(Vec<PathBuf>),
    ThemePicked(String),
    Scrolled(Viewport),
    HunkClicked(u32),
    Do(Action),
    Applied(PathBuf, Result<Applied, String>),
    RewordEdited(text_editor::Action),
    RewordApply,
    RewordCancel,
    SuggestionReady(u64, PathBuf, u64, Result<Suggested, String>),
    SpinnerFrame,
    SuggestionApprove,
    SuggestionReject,
    SuggestionRetry,
    ToggleIncludeFunction,
    AiKeyInput(String),
    AiKeySave,
    AiKeyRemove,
    AiModelInput(String),
    AiModelsLoaded(ai::Provider, Result<Vec<String>, String>),
    AiPresetPicked(Preset),
    AiCustomEdited(text_editor::Action),
    AiIncludeFunction(bool),
    GoThroughAll,
    StopReview,
    Previous,
    Next,
    Deselect,
    Undo,
    Undone(PathBuf, Result<Loaded, String>),
    FixAll,
    CleanFolder,
    PlanCancel,
    PlanApply,
    PlanApplied(PlanOutcome),
    PlanToggleKind(bool, bool),
    PlanToggleFile(usize, bool),
    PlanToggleItem(usize, usize, bool),
    PlanExpand(usize),
    UndoFolder,
    FolderUndone(usize, usize),
    ScanBatch(u64, Vec<ScannedFile>),
    ScanDone(u64),
    ScanUpdated(u64, Vec<ScannedFile>, Vec<PathBuf>),
    ToggleSettings,
    SetBool(BoolField, bool),
    SetWidth(String),
    SetLines(String),
    ProtectEdited(text_editor::Action),
    DeleteEdited(text_editor::Action),
    FolderSettings(bool),
    SaveRules,
    DiscardRules,
    ImpactReady(u64, Option<RuleImpact>),
    RuleSampleEdited(text_editor::Action),
    GitRead(PathBuf, Option<Repo>),
    WindowFocused,
    ToggleOnlyChanged,
    OpenFind,
    CloseFind,
    FindInput(String),
    FindToggle(FindOption),
    FindSubmit,
    FindStep(bool),
    FindResults(u64, Arc<Vec<FindMatch>>),
    EscapeInTextBox,
    OpenSearch,
    ToggleSearchPanel,
    SearchInput(String),
    SearchToggle(FindOption),
    SearchBatch(u64, Vec<SearchFile>),
    SearchDone(u64),
    SearchScrolled(Viewport),
    SearchToggleFile(PathBuf),
    SearchOpenHit(usize, usize),
    CheckForUpdates,
    AutoCheckForUpdates,
    UpdateChecked(bool, Result<Option<Release>, String>),
    UpdateDownload(Release),
    UpdateProgress(update::Progress),
    UpdateRestart,
    SetAutoUpdate(bool),
    CloseWhatsNew,
    FixModePicked(FixMode),
    AiSuggestAhead(bool),
    SuggestedAhead(u64, PathBuf, u64, Result<Suggested, String>),
    AiProviderPicked(Provider),
    AiCustomUrlInput(String),
    AiCustomUrlSubmit,
    OpenPalette,
    ClosePalette,
    PaletteInput(String),
    PaletteMove(i32),
    PaletteRun(usize),
    Frame(Instant),
    TakeShot,
    Shot(window::Screenshot),
}

fn spawn_work<A: Send + 'static>(work: impl FnOnce() -> A + Send + 'static) -> oneshot::Receiver<A> {
    let (sender, receiver) = oneshot::channel();
    std::thread::spawn(move || {
        let _ = sender.send(work());
    });
    receiver
}

fn background<A, T>(work: impl FnOnce() -> A + Send + 'static, done: impl FnOnce(A) -> T + Send + 'static) -> Task<T>
where
    A: Send + 'static,
    T: Send + 'static,
{
    let result = spawn_work(work);
    let mut done = Some(done);
    Task::stream(iced::futures::stream::once(result).filter_map(move |value| {
        std::future::ready(value.ok().zip(done.take()).map(|(value, done)| done(value)))
    }))
}

fn scroll_code(x: f32, y: f32) -> Task<Message> {
    operation::scroll_to(Id::new(CODE_SCROLL), AbsoluteOffset { x, y })
}

fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgb(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
    )
}

fn load_highlighter(include_editors: bool) -> Result<Arc<Highlighter>, String> {
    let catalog = vscode::scan(include_editors);
    let (highlighter, report) = Highlighter::load(catalog)?;
    println!("loaded grammars and themes in {:?}", report.elapsed);
    for error in report.grammar_errors.iter().chain(&report.theme_errors) {
        println!("  skipped {error}");
    }
    Ok(Arc::new(highlighter))
}

impl App {
    pub fn boot(options: Options) -> (Self, Task<Message>) {
        let include_editors = !options.standalone;
        let mut task = background(
            move || { load_highlighter(include_editors) },
            Message::HighlighterReady,
        );
        let session = session::load();
        let saved_placement = session.window;
        if let Some(saved) = saved_placement {
            task = Task::batch([
                task,
                window::latest()
                    .and_then(window::raw_id::<Message>)
                    .then(move |handle| {
                        background(move || { window_state::apply(handle, saved) }, |()| ()).discard()
                    })
                    .chain(window::latest().and_then(|id| window::set_mode(id, window::Mode::Windowed))),
            ]);
        }
        if options.shot.is_some() {
            let delay = options.delay;
            task = Task::batch([
                task,
                background(move || std::thread::sleep(Duration::from_millis(delay)), |()| Message::TakeShot),
            ]);
        }
        let ai = AiSettings::load();
        let ai_has_key = ai::has_key(ai.provider);
        let mut app = Self {
            options,
            highlighter: None,
            theme_name: String::new(),
            theme: Theme::Dark,
            palette: Palette {
                foreground: Color::WHITE,
                line_number: Color::from_rgb(0.5, 0.5, 0.5),
            },
            background: Color::from_rgb(0.12, 0.12, 0.12),
            root: None,
            expanded: HashSet::new(),
            entries: Vec::new(),
            tabs: Vec::new(),
            active: None,
            reword: None,
            busy: false,
            status: "Loading grammars and themes…".into(),
            next_generation: 1,
            auto: None,
            rules: Arc::new(Rules::new(&Settings::default())),
            settings: Settings::default(),
            folder_settings: false,
            show_settings: false,
            plan: None,
            folder_undo: Vec::new(),
            scan: HashMap::new(),
            scan_running: false,
            scan_generation: 0,
            scan_abort: None,
            scan_stale: false,
            last_recount: Instant::now(),
            counts: HashMap::new(),
            ignored_store: HashMap::new(),
            restore: None,
            pending_review: None,
            protect_patterns: text_editor::Content::new(),
            delete_patterns: text_editor::Content::new(),
            width_input: String::new(),
            lines_input: String::new(),
            menu: None,
            maximized: false,
            last_title_press: None,
            title_armed: false,
            ctrl_held: false,
            show_sidebar: true,
            icon_theme: None,
            icon_theme_name: icons::BUILT_IN.to_owned(),
            icon_theme_names: vec![icons::BUILT_IN.to_owned()],
            entry_icons: Vec::new(),
            ai_custom: text_editor::Content::with_text(&ai.custom),
            ai,
            ai_key_input: String::new(),
            ai_has_key,
            ai_models: Vec::new(),
            ai_models_error: None,
            suggestion: None,
            suggest_seq: 0,
            saved_settings: Settings::default(),
            edited_rules: Arc::new(Rules::new(&Settings::default())),
            rule_impact: None,
            impact_seq: Arc::new(AtomicU64::new(0)),
            rule_sample: text_editor::Content::new(),
            rule_sample_result: None,
            repo: None,
            repo_dirs: HashSet::new(),
            git_running: false,
            git_again: false,
            only_changed: session.only_changed,
            find_open: false,
            find_query: Query::default(),
            find_matches: Arc::new(Vec::new()),
            find_current: None,
            find_error: None,
            find_seq: 0,
            find_reveal: false,
            find_goto: None,
            shift_held: false,
            code_view: Size::new(900.0, 600.0),
            side_panel: SidePanel::Files,
            search_query: Query::default(),
            search_files: Vec::new(),
            search_rows: Vec::new(),
            search_total: 0,
            search_running: false,
            search_error: None,
            search_collapsed: HashSet::new(),
            search_seq: 0,
            search_abort: None,
            search_replacing: false,
            search_scroll: (0.0, 600.0),
            confirm_exit: None,
            placement: saved_placement,
            update: UpdateState::Idle,
            auto_update: !session.manual_updates,
            update_restart: false,
            update_abort: None,
            update_notes: None,
            whats_new: session
                .whats_new
                .as_ref()
                .filter(|(version, _)| version == update::VERSION)
                .map(|(version, notes)| (version.clone(), update::note_lines(notes))),
            ahead: HashMap::new(),
            ahead_epoch: 0,
            ahead_paused: None,
            suggest_cache: HashMap::new(),
            spinner_start: Instant::now(),
            palette_open: false,
            palette_query: String::new(),
            palette_selected: 0,
            fix_mode: FixMode::default(),
        };
        app.load_settings();
        let mut session = session;
        app.ignored_store = std::mem::take(&mut session.ignored);
        if app.options.folder.is_some() || app.options.open.is_some() {
            session = Session::default();
        }
        if let Some(folder) = app.options.folder.clone() {
            app.set_root(folder);
        } else if let Some(folder) = session.folder.clone().filter(|f| f.is_dir()) {
            app.set_root(folder);
        }
        app.restore = Some(session);
        (app, task)
    }

    pub fn title(&self) -> String {
        let folder = self.root.as_ref().and_then(|root| root.file_name()).map(|n| n.to_string_lossy());
        match (folder, self.tab().map(|tab| tab.name.as_str())) {
            (Some(folder), Some(file)) => format!("{folder} - CommentCleanup - {file}"),
            (Some(folder), None) => format!("{folder} - CommentCleanup"),
            (None, Some(file)) => format!("CommentCleanup - {file}"),
            (None, None) => "CommentCleanup".to_owned(),
        }
    }

    pub fn theme(&self) -> Theme {
        self.theme.clone()
    }

    fn set_root(&mut self, folder: PathBuf) {
        self.expanded.clear();
        self.root = Some(folder);
        self.forget_folder_state();
        self.rebuild_tree();
        self.load_settings();
        self.save_session();
    }

    fn save_session(&mut self) {
        if self.restore.is_some() {
            return;
        }
        let session = Session {
            folder: self.root.clone(),
            tabs: self.tabs.iter().map(|tab| tab.path.clone()).collect(),
            active: self.active,
            ignored: self.ignored_store.clone(),
            window: self.placement,
            only_changed: self.only_changed,
            manual_updates: !self.auto_update,
            whats_new: self.update_notes.clone(),
        };
        if let Err(error) = session::save(&session) {
            self.status = error;
        }
    }

    fn start_scan(&mut self) -> Task<Message> {
        if let Some(handle) = self.scan_abort.take() {
            handle.abort();
        }
        let (Some(root), Some(highlighter)) = (self.root.clone(), self.highlighter.clone()) else {
            return Task::none();
        };
        self.scan.clear();
        self.counts.clear();
        self.scan_running = true;
        self.scan_stale = false;
        self.scan_generation += 1;
        let generation = self.scan_generation;
        let stream = scan::scan_folder(highlighter, self.rules.clone(), root, self.theme_name.clone());
        let (task, handle) = Task::run(stream, move |batch| Message::ScanBatch(generation, batch))
            .chain(Task::done(Message::ScanDone(generation)))
            .abortable();
        self.scan_abort = Some(handle.abort_on_drop());
        Task::batch([task, self.refresh_git()])
    }

    fn forget_folder_state(&mut self) {
        self.repo = None;
        self.repo_dirs.clear();
        self.search_abort = None;
        self.search_running = false;
        self.search_seq += 1;
        self.search_files.clear();
        self.search_rows.clear();
        self.search_total = 0;
        self.search_collapsed.clear();
        self.search_error = None;
    }

    fn refresh_git(&mut self) -> Task<Message> {
        let Some(root) = self.root.clone() else {
            return Task::none();
        };
        if self.git_running {
            self.git_again = true;
            return Task::none();
        }
        self.git_running = true;
        let reply = root.clone();
        background(move || { git::read(&root) }, move |repo| Message::GitRead(reply.clone(), repo))
    }

    /// `None` means every comment in this file counts. Otherwise only those on these changed lines do, and a file with no changes has none.
    fn scope_for(&self, path: &Path) -> Option<Option<&Changed>> {
        if !self.only_changed || !self.root.as_ref().is_some_and(|root| path.starts_with(root)) {
            return None;
        }
        let repo = self.repo.as_ref()?;
        Some(repo.files.get(path).map(|file| &file.changed))
    }

    fn git_state(&self, path: &Path) -> Option<FileState> {
        self.repo.as_ref()?.files.get(path).map(|file| file.state)
    }

    fn rules_dirty(&self) -> bool {
        self.settings != self.saved_settings
    }

    fn rules_edited(&mut self) -> Task<Message> {
        self.edited_rules = Arc::new(Rules::new(&self.settings));
        self.update_rule_sample();
        self.test_rules()
    }

    fn update_rule_sample(&mut self) {
        let comment = self.rule_sample.text();
        if comment.trim().is_empty() {
            self.rule_sample_result = None;
            return;
        }
        let Some(highlighter) = &self.highlighter else {
            return;
        };
        let language = self
            .tab()
            .map(|tab| tab.doc.language.as_str())
            .filter(|language| highlighter.syntax_for(language).is_some())
            .unwrap_or("javascript");
        let verdict = rules::try_comment(&self.edited_rules, &comment, highlighter.syntax_for(language));
        self.rule_sample_result = Some(match verdict {
            Some(Decision::Squash(text)) => format!("Squashed into: {text}"),
            Some(decision) => decision.summary(),
            None => format!("That isn't written as a {language} comment."),
        });
    }

    fn test_rules(&mut self) -> Task<Message> {
        let seq = self.impact_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(highlighter) = self.highlighter.clone() else {
            return Task::none();
        };
        let root = self.root.as_deref();
        let label = |path: &Path| {
            root.and_then(|root| path.strip_prefix(root).ok()).unwrap_or(path).to_string_lossy().into_owned()
        };
        let mut files: Vec<ImpactFile> = self
            .scan
            .values()
            .map(|file| ImpactFile {
                path: file.path.clone(),
                label: label(&file.path),
                language: file.language.clone(),
                hunks: file.hunks.clone(),
            })
            .collect();
        files.extend(self.tabs.iter().filter(|tab| !self.scan.contains_key(&tab.path)).map(|tab| ImpactFile {
            path: tab.path.clone(),
            label: tab.name.clone(),
            language: tab.doc.language.clone(),
            hunks: Arc::new(tab.doc.hunks.clone()),
        }));
        files.sort_by_key(|file| file.label.to_lowercase());
        let (saved, edited) = (self.rules.clone(), self.edited_rules.clone());
        let ignored = self.ignored_store.clone();
        let latest = self.impact_seq.clone();
        background(
            move || {
                std::thread::sleep(IMPACT_DELAY);
                let still_wanted = || latest.load(Ordering::SeqCst) == seq;
                if !still_wanted() {
                    return None;
                }
                scan::rule_impact(&highlighter, &files, &saved, &edited, &ignored, still_wanted)
            },
            move |impact| Message::ImpactReady(seq, impact),
        )
    }

    /// Finds again in the active tab
    fn run_find(&mut self, reveal: bool) -> Task<Message> {
        self.find_seq += 1;
        self.find_reveal |= reveal;
        let doc = self.tab().filter(|_| self.find_open).map(|tab| tab.doc.clone());
        let compiled = self.find_query.compile();
        self.find_error = compiled.as_ref().err().cloned();
        match (doc, compiled.ok().flatten()) {
            (Some(doc), Some(regex)) => {
                let seq = self.find_seq;
                background(move || { Arc::new(find::in_document(&doc, &regex)) }, move |matches| {
                    Message::FindResults(seq, matches)
                })
            }
            _ => {
                self.find_matches = Arc::new(Vec::new());
                self.find_current = None;
                Task::none()
            }
        }
    }

    fn reveal_match(&self, index: usize) -> Task<Message> {
        let (Some(tab), Some(found)) = (self.tab(), self.find_matches.get(index)) else {
            return Task::none();
        };
        let Some(line) = tab.doc.lines.get(found.line as usize) else {
            return Task::none();
        };
        let text: String = line.segments.iter().map(|segment| segment.text.as_str()).collect();
        let column = text.get(..found.start as usize).map_or(0, |before| before.chars().count());
        let width = text
            .get(found.start as usize..found.end as usize)
            .map_or(0, |matched| matched.chars().count());
        let line_y = found.line as f32 * LINE_HEIGHT;
        let (view_w, view_h) = (self.code_view.width, self.code_view.height);
        let top_of_view = tab.offset.y + FIND_BAR_HEIGHT + LINE_HEIGHT;
        let bottom_of_view = tab.offset.y + view_h - LINE_HEIGHT * 2.0;
        let y = if line_y < top_of_view || line_y > bottom_of_view {
            (line_y - view_h / 2.0).max(0.0)
        } else {
            tab.offset.y
        };
        let left = column as f32 * CODE_ADVANCE;
        let right = (column + width) as f32 * CODE_ADVANCE + CODE_GUTTER;
        let x = if left < tab.offset.x || right > tab.offset.x + view_w - SCROLLBAR_ROOM * 2.0 {
            (right - view_w / 2.0).max(0.0)
        } else {
            tab.offset.x
        };
        if x == tab.offset.x && y == tab.offset.y {
            return Task::none();
        }
        scroll_code(x, y)
    }

    fn start_search(&mut self) -> Task<Message> {
        self.search_abort = None;
        self.search_seq += 1;
        let compiled = self.search_query.compile();
        self.search_error = compiled.as_ref().err().cloned();
        let (Some(root), Some(regex)) = (self.root.clone(), compiled.ok().flatten()) else {
            self.search_running = false;
            self.search_files.clear();
            self.search_rows.clear();
            self.search_total = 0;
            return Task::none();
        };
        self.search_running = true;
        self.search_replacing = true;
        let seq = self.search_seq;
        let (task, handle) = Task::run(find::search_folder(root, regex), move |batch| Message::SearchBatch(seq, batch))
            .chain(Task::done(Message::SearchDone(seq)))
            .abortable();
        self.search_abort = Some(handle.abort_on_drop());
        task
    }

    fn search_to_top(&mut self) -> Task<Message> {
        self.search_scroll.0 = 0.0;
        operation::scroll_to(Id::new(SEARCH_SCROLL), AbsoluteOffset { x: 0.0, y: 0.0 })
    }

    fn rebuild_search_rows(&mut self) {
        self.search_total = self.search_files.iter().map(|file| file.hits.len()).sum();
        self.search_rows.clear();
        for (index, file) in self.search_files.iter().enumerate() {
            self.search_rows.push(SearchRow::File(index));
            if !self.search_collapsed.contains(&file.path) {
                self.search_rows.extend((0..file.hits.len()).map(|hit| SearchRow::Hit(index, hit)));
            }
        }
    }

    fn exit_concerns(&self) -> Vec<&'static str> {
        let mut concerns = Vec::new();
        let reworded = match (&self.reword, self.selection()) {
            (Some(content), Some((_, _, hunk))) => content.text().trim_end() != hunk.raw,
            _ => false,
        };
        if reworded {
            concerns.push("A Reword you haven't applied.");
        }
        if self.rules_dirty() {
            concerns.push("Rule changes you haven't saved.");
        }
        if self.busy {
            concerns.push("A cleanup that is still being written.");
        }
        concerns
    }

    fn quit(&self) -> Task<Message> {
        window::latest().and_then(window::raw_id::<Message>).map(Message::Quit)
    }

    fn check_for_updates(&mut self, manual: bool) -> Task<Message> {
        let busy = matches!(
            self.update,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
        );
        if busy || !update::available() {
            return Task::none();
        }
        self.update = UpdateState::Checking;
        background(move || { update::check() }, move |result| Message::UpdateChecked(manual, result))
    }

    fn start_download(&mut self, release: Release) -> Task<Message> {
        self.update = UpdateState::Downloading {
            version: release.version.clone(),
            percent: 0,
            notes: release.notes.clone(),
        };
        let (task, handle) = Task::run(update::download(release), Message::UpdateProgress).abortable();
        self.update_abort = Some(handle.abort_on_drop());
        task
    }

    fn rebuild_plan(&mut self) {
        if let Some(folder) = self.plan.as_ref().map(|plan| plan.folder) {
            self.plan = Some(if folder {
                self.folder_plan()
            } else {
                self.file_plan().unwrap_or(Plan { folder: false, files: Vec::new() })
            });
        }
    }

    fn stop_scan(&mut self) {
        if let Some(handle) = self.scan_abort.take() {
            handle.abort();
        }
        self.scan_generation += 1;
        self.scan_running = false;
        self.scan.clear();
        self.counts.clear();
    }

    fn reviewable(&self, file: &ScannedFile) -> usize {
        let ignored = self.ignored_store.get(&file.path);
        let scope = self.scope_for(&file.path);
        file.hunks
            .iter()
            .zip(&file.decisions)
            .filter(|(hunk, decision)| {
                !decision.is_protected()
                    && !ignored.is_some_and(|keys| keys.contains(&hunk.key))
                    && in_scope(scope, hunk)
            })
            .count()
    }

    fn recount(&mut self) {
        let Some(root) = &self.root else {
            return;
        };
        let mut counts: HashMap<PathBuf, usize> = HashMap::new();
        for (path, file) in &self.scan {
            let count = self.reviewable(file);
            if count == 0 {
                continue;
            }
            for ancestor in path.ancestors() {
                *counts.entry(ancestor.to_path_buf()).or_default() += count;
                if ancestor == root {
                    break;
                }
            }
        }
        self.counts = counts;
        self.last_recount = Instant::now();
    }

    fn refresh_scan_decisions(&mut self) {
        if !self.scan_stale {
            return;
        }
        self.scan_stale = false;
        let Some(highlighter) = self.highlighter.clone() else {
            return;
        };
        for file in self.scan.values_mut() {
            let syntax = highlighter.syntax_for(&file.language);
            file.decisions = file.hunks.iter().map(|hunk| self.rules.decide(hunk, syntax)).collect();
        }
        self.recount();
    }

    fn folder_plan(&self) -> Plan {
        let root = self.root.as_deref();
        let mut files: Vec<PlanFile> = self
            .scan
            .values()
            .filter_map(|file| {
                let ignored = self.ignored_store.get(&file.path);
                let scope = self.scope_for(&file.path);
                let items: Vec<PlanItem> = file
                    .hunks
                    .iter()
                    .zip(&file.decisions)
                    .filter(|(hunk, _)| !ignored.is_some_and(|keys| keys.contains(&hunk.key)))
                    .filter(|(hunk, _)| in_scope(scope, hunk))
                    .filter_map(|(hunk, decision)| plan_item(hunk, decision, false))
                    .collect();
                (!items.is_empty()).then(|| PlanFile {
                    label: root
                        .and_then(|root| file.path.strip_prefix(root).ok())
                        .unwrap_or(&file.path)
                        .to_string_lossy()
                        .into_owned(),
                    path: file.path.clone(),
                    hash: file.hash,
                    items,
                    expanded: false,
                })
            })
            .collect();
        files.sort_by_key(|file| file.label.to_lowercase());
        if let [only] = files.as_mut_slice() {
            only.expanded = true;
        }
        Plan { folder: true, files }
    }

    fn file_plan(&self) -> Option<Plan> {
        let tab = self.tab()?;
        let scope = self.scope_for(&tab.path);
        let syntax = self.highlighter.as_ref().and_then(|h| h.syntax_for(&tab.doc.language));
        let items: Vec<PlanItem> = tab
            .doc
            .hunks
            .iter()
            .zip(&tab.decisions)
            .filter(|(hunk, _)| !tab.ignored.contains(&hunk.key))
            .filter(|(hunk, _)| in_scope(scope, hunk))
            .filter_map(|(hunk, decision)| self.fix_mode.item(hunk, decision, syntax))
            .collect();
        (!items.is_empty()).then(|| Plan {
            folder: false,
            files: vec![PlanFile {
                path: tab.path.clone(),
                label: tab.name.clone(),
                hash: tab.hash,
                items,
                expanded: true,
            }],
        })
    }

    fn next_review_file(&self) -> Option<PathBuf> {
        let current = &self.tab()?.path;
        if !current.starts_with(self.root.as_ref()?) {
            return None;
        }
        self.scan
            .values()
            .filter(|file| file.path > *current && self.reviewable(file) > 0)
            .map(|file| file.path.clone())
            .min()
    }

    fn review_next_file(&mut self) -> Option<Task<Message>> {
        let next = self.next_review_file()?;
        if let Some(tab) = self.tab_mut() {
            tab.review = None;
            tab.selected = None;
        }
        self.reword = None;
        if let Some(index) = self.tabs.iter().position(|tab| tab.path == next) {
            let scroll = self.activate(index);
            return Some(scroll.chain(self.update(Message::GoThroughAll)));
        }
        self.pending_review = Some(next.clone());
        Some(self.open_file(next))
    }

    fn load_settings(&mut self) {
        let folder = self
            .root
            .as_deref()
            .map(rules::workspace_settings_path)
            .and_then(|path| rules::read_settings(&path));
        self.folder_settings = folder.is_some();
        if self.rules_dirty() {
            self.status = "Unsaved rule changes were discarded.".into();
        }
        self.saved_settings = folder.unwrap_or_else(|| {
            rules::app_settings_path()
                .and_then(|path| rules::read_settings(&path))
                .unwrap_or_default()
        });
        self.settings = self.saved_settings.clone();
        self.reset_rule_inputs();
        self.apply_rules();
    }

    fn reset_rule_inputs(&mut self) {
        self.protect_patterns =
            text_editor::Content::with_text(&self.settings.protect_patterns.join("\n"));
        self.delete_patterns =
            text_editor::Content::with_text(&self.settings.delete_patterns.join("\n"));
        self.width_input = self.settings.squash_max_width.to_string();
        self.lines_input = self.settings.squash_max_lines.to_string();
    }

    fn apply_rules(&mut self) {
        self.rules = Arc::new(Rules::new(&self.saved_settings));
        self.edited_rules = Arc::new(Rules::new(&self.settings));
        self.scan_stale = !self.scan.is_empty();
        for index in 0..self.tabs.len() {
            self.refresh_decisions(index);
        }
    }

    fn refresh_decisions(&mut self, index: usize) {
        if self.highlighter.is_none() {
            return;
        }
        let scope = self.scope_for(&self.tabs[index].path).map(|changed| changed.cloned());
        let Some(highlighter) = &self.highlighter else {
            return;
        };
        let tab = &mut self.tabs[index];
        let syntax = highlighter.syntax_for(&tab.doc.language);
        tab.decisions = tab.doc.hunks.iter().map(|h| self.rules.decide(h, syntax)).collect();
        tab.dimmed = tab.ignored.clone();
        tab.dimmed.extend(
            tab.doc
                .hunks
                .iter()
                .filter(|hunk| !in_scope(scope.as_ref().map(Option::as_ref), hunk))
                .map(|hunk| hunk.key),
        );
        tab.dimmed.extend(
            tab.doc
                .hunks
                .iter()
                .zip(&tab.decisions)
                .filter(|(_, decision)| decision.is_protected())
                .map(|(hunk, _)| hunk.key),
        );
    }

    fn save_rules(&mut self) -> Task<Message> {
        self.saved_settings = self.settings.clone();
        self.apply_rules();
        let path = if self.folder_settings {
            self.root.as_deref().map(rules::workspace_settings_path)
        } else {
            rules::app_settings_path()
        };
        self.status = "Rules saved.".into();
        if let Some(path) = path {
            if let Err(error) = rules::write_settings(&path, &self.saved_settings) {
                self.status = error;
            }
        }
        self.refresh_scan_decisions();
        self.update_rule_sample();
        self.test_rules()
    }

    fn rebuild_tree(&mut self) {
        if let Some(root) = &self.root {
            self.entries = files::list_tree(root, &self.expanded);
        }
        self.refresh_icons();
    }

    fn refresh_icons(&mut self) {
        let [r, g, b, _] = self.background.into_rgba8();
        let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 128.0;
        self.entry_icons = self.entries.iter().map(|e| self.icon_for(e, light)).collect();
    }

    fn icon_for(&self, entry: &Entry, light: bool) -> Icon {
        let expanded = self.expanded.contains(&entry.path);
        let themed = self.icon_theme.as_ref().and_then(|icons| {
            let language = match &self.highlighter {
                Some(highlighter) if !entry.is_dir => highlighter.language_for(&entry.path),
                _ => String::new(),
            };
            icons.theme.resolve(&entry.name, &language, entry.is_dir, expanded, light)
        });
        themed.unwrap_or_else(|| icons::built_in(&entry.name, entry.is_dir, expanded))
    }

    fn load_icon_theme(&self, entry: IconThemeEntry) -> Task<Message> {
        let label = entry.label.clone();
        let text_color = self.palette.foreground;
        background(
            move || { IconTheme::load(&entry, text_color).map(Arc::new) },
            move |result| Message::IconsLoaded(label.clone(), result),
        )
    }

    fn generation(&mut self) -> u64 {
        self.next_generation += 1;
        self.next_generation
    }

    fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active?)
    }

    fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active?)
    }

    fn apply_theme(&mut self, name: String) {
        let Some(highlighter) = &self.highlighter else {
            return;
        };
        let Some(colors) = highlighter.palettes.get(&name.to_lowercase()) else {
            return;
        };
        let background = color(parse_hex(&colors.background));
        let foreground = color(parse_hex(&colors.foreground));
        self.palette = Palette {
            foreground,
            line_number: color(parse_hex(&colors.line_number)),
        };
        self.background = background;
        self.theme = Theme::custom(
            name.clone(),
            iced::theme::Palette {
                background,
                text: foreground,
                primary: Color::from_rgb8(0x0e, 0x63, 0x9c),
                success: Color::from_rgb8(0x38, 0x8a, 0x3c),
                warning: Color::from_rgb8(0xcc, 0xa7, 0x00),
                danger: Color::from_rgb8(0xc7, 0x4e, 0x39),
            },
        );
        self.theme_name = name;
        self.refresh_icons();
    }

    fn load_tab(&mut self, index: usize, known: Option<u64>) -> Task<Message> {
        let Some(highlighter) = self.highlighter.clone() else {
            return Task::none();
        };
        let theme = self.theme_name.clone();
        let tab = &mut self.tabs[index];
        tab.load_seq += 1;
        let seq = tab.load_seq;
        let path = tab.path.clone();
        let reply_path = path.clone();
        background(
            move || { files::load(&highlighter, &path, &theme, known) },
            move |result| Message::Loaded(reply_path.clone(), seq, result),
        )
    }

    fn open_file(&mut self, path: PathBuf) -> Task<Message> {
        if let Some(index) = self.tabs.iter().position(|t| t.path == path) {
            let task = self.activate(index);
            self.save_session();
            return task;
        }
        let generation = self.generation();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ignored = self.ignored_store.get(&path).cloned().unwrap_or_default();
        self.tabs.push(Tab {
            path,
            name,
            doc: Arc::new(Document::empty()),
            hash: 0,
            generation,
            read_only: None,
            load_seq: 0,
            offset: AbsoluteOffset::default(),
            selected: None,
            ignored,
            decisions: Vec::new(),
            dimmed: HashSet::new(),
            undo: Vec::new(),
            review: None,
            function_override: HashMap::new(),
        });
        let index = self.tabs.len() - 1;
        let task = self.activate(index);
        self.save_session();
        Task::batch([task, self.load_tab(index, None)])
    }

    fn activate(&mut self, index: usize) -> Task<Message> {
        self.active = Some(index);
        self.show_settings = false;
        self.refresh_scan_decisions();
        self.reword = None;
        self.suggestion = None;
        self.forget_ahead();
        let offset = self.tabs[index].offset;
        Task::batch([scroll_code(offset.x, offset.y), self.run_find(false)])
    }

    fn select(&mut self, index: Option<usize>) {
        self.reword = None;
        self.suggestion = None;
        if let Some(tab) = self.tab_mut() {
            tab.selected = index;
        }
    }

    fn jump_to(&mut self, index: usize) -> Task<Message> {
        self.select(Some(index));
        let Some(tab) = self.tab() else {
            return Task::none();
        };
        let y = tab.doc.hunks[index].first.saturating_sub(CONTEXT_LINES) as f32 * LINE_HEIGHT;
        Task::batch([scroll_code(0.0, y), self.arrive_ahead()])
    }

    fn forget_ahead(&mut self) {
        self.ahead.clear();
        self.ahead_epoch += 1;
    }

    fn suggesting_ahead(&self) -> bool {
        self.ai.suggest_ahead
            && self.ai_ready()
            && !self.ai.model.trim().is_empty()
            && self.ai.instructions().is_some()
            && self.tab().is_some_and(|tab| tab.review.is_some() && tab.read_only.is_none())
    }

    fn suggestion_request(&self, tab: &Tab, hunk: &Hunk, rejected: Vec<String>) -> Option<(Arc<Highlighter>, ai::Request)> {
        let highlighter = self.highlighter.clone()?;
        let request = ai::Request {
            endpoint: self.ai.endpoint(),
            path: tab.path.clone(),
            theme: self.theme_name.clone(),
            doc: tab.doc.clone(),
            hunk: hunk.clone(),
            syntax: highlighter.syntax_for(&tab.doc.language).cloned(),
            model: self.ai.model.trim().to_owned(),
            instructions: self.ai.instructions()?,
            include_function: self.includes_function(tab, hunk.key),
            rejected,
        };
        Some((highlighter, request))
    }

    fn suggestion_settings(&self, tab: &Tab, key: u64) -> (String, String, bool) {
        (
            format!("{}/{}", self.ai.provider.name(), self.ai.model.trim()),
            self.ai.instructions().unwrap_or_default(),
            self.includes_function(tab, key),
        )
    }

    fn suggestion_gate(&self, tab: &Tab, hunk: &Hunk, retrying: bool) -> SuggestGate {
        let Some(entry) = self.suggest_cache.get(&(tab.path.clone(), hunk.key)) else {
            return SuggestGate::Ask;
        };
        if !retrying
            && entry.settings == self.suggestion_settings(tab, hunk.key)
            && let Some(reply) = &entry.reply
        {
            return SuggestGate::Cached(reply.clone());
        }
        match SUGGEST_COOLDOWN.checked_sub(entry.asked.elapsed()) {
            Some(left) if !left.is_zero() => SuggestGate::Wait(left.as_secs() + 1),
            _ => SuggestGate::Ask,
        }
    }

    fn remember_reply(&mut self, path: PathBuf, key: u64, result: &Result<Suggested, String>) {
        if let (Ok(suggested), Some(entry)) = (result, self.suggest_cache.get_mut(&(path, key))) {
            entry.reply = Some(suggested.clone());
        }
    }

    fn top_up_ahead(&mut self) -> Task<Message> {
        if !self.suggesting_ahead() {
            return Task::none();
        }
        let paused = self.ahead_paused.is_some_and(|until| Instant::now() < until);
        let in_flight = self.ahead.values().filter(|state| matches!(state, AheadState::Loading)).count();
        let Some(tab) = self.tab() else {
            return Task::none();
        };
        let Some(selected) = tab.selected else {
            return Task::none();
        };
        let candidates: Vec<Hunk> = tab.doc.hunks[selected..]
            .iter()
            .filter(|hunk| !tab.dimmed.contains(&hunk.key))
            .take(AHEAD_COMMENTS)
            .filter(|hunk| !self.ahead.contains_key(&hunk.key))
            .cloned()
            .collect();
        let epoch = self.ahead_epoch;
        let mut room = AHEAD_AT_ONCE.saturating_sub(in_flight);
        let mut tasks = Vec::new();
        for hunk in candidates {
            let key = hunk.key;
            let Some(tab) = self.tab() else {
                break;
            };
            match self.suggestion_gate(tab, &hunk, false) {
                SuggestGate::Cached(reply) => {
                    self.ahead.insert(key, AheadState::Ready(reply));
                }
                SuggestGate::Ask if room > 0 && !paused => {
                    let path = tab.path.clone();
                    let settings = self.suggestion_settings(tab, key);
                    let Some((highlighter, request)) = self.suggestion_request(tab, &hunk, Vec::new()) else {
                        continue;
                    };
                    room -= 1;
                    self.suggest_cache.insert(
                        (path.clone(), key),
                        CachedSuggestion { asked: Instant::now(), settings, reply: None },
                    );
                    self.ahead.insert(key, AheadState::Loading);
                    tasks.push(background(move || { ai::suggest(&highlighter, request) }, move |result| {
                        Message::SuggestedAhead(epoch, path, key, result)
                    }));
                }
                _ => {}
            }
        }
        Task::batch(tasks)
    }

    fn arrive_ahead(&mut self) -> Task<Message> {
        if !self.suggesting_ahead() {
            return Task::none();
        }
        let task = self.top_up_ahead();
        let Some(key) = self.selected_hunk().map(|hunk| hunk.key) else {
            return task;
        };
        if let Some(fetched) = self.ahead.get(&key) {
            let state = match fetched {
                AheadState::Loading => SuggestionState::Loading,
                AheadState::Ready(suggested) => SuggestionState::Ready(suggested.clone()),
                AheadState::Failed(error) => SuggestionState::Failed(error.clone()),
            };
            self.suggestion = Some(Suggestion { key, state, rejected: Vec::new() });
        }
        task
    }

    fn palette_matches(&self) -> Vec<(&'static str, &'static str, Message)> {
        let commands: Vec<_> = [Menu::File, Menu::Edit, Menu::View]
            .into_iter()
            .flat_map(|menu| self.menu_entries(menu))
            .filter_map(|entry| match entry {
                MenuEntry::Item(label, shortcut, Some(message)) if !matches!(message, Message::OpenPalette) => {
                    Some((label, shortcut, message))
                }
                _ => None,
            })
            .collect();
        palette::rank(&self.palette_query, commands.iter().map(|(label, ..)| *label))
            .into_iter()
            .map(|index| commands[index].clone())
            .collect()
    }

    fn selected_hunk(&self) -> Option<Hunk> {
        self.selection().map(|(_, _, hunk)| hunk.clone())
    }

    fn selection(&self) -> Option<(&Tab, usize, &Hunk)> {
        let tab = self.tab()?;
        let index = tab.selected?;
        Some((tab, index, tab.doc.hunks.get(index)?))
    }

    fn available(&self) -> Available {
        let Some((tab, index, hunk)) = self.selection() else {
            return Available::default();
        };
        let can_edit = !self.busy && tab.read_only.is_none();
        let syntax = self
            .highlighter
            .as_ref()
            .and_then(|highlighter| highlighter.syntax_for(&tab.doc.language));
        Available {
            fix: can_edit && tab.decisions.get(index).is_some_and(Decision::is_fix),
            ignore: !self.busy,
            squash: can_edit && cleanup::squash(hunk, syntax).is_some(),
            delete: can_edit,
            reword: can_edit,
            suggest: can_edit
                && !tab.decisions.get(index).is_some_and(Decision::is_protected)
                && !matches!(self.suggestion, Some(Suggestion { state: SuggestionState::Loading, .. })),
        }
    }

    /// With a suggestion open, R means Retry.
    fn reword_shortcut(&self) -> &'static str {
        if self.suggestion.is_some() { "" } else { "R" }
    }

    fn includes_function(&self, tab: &Tab, key: u64) -> bool {
        tab.function_override.get(&key).copied().unwrap_or(self.ai.include_function)
    }

    fn save_ai(&mut self) {
        self.forget_ahead();
        if let Err(error) = self.ai.save() {
            self.status = error;
        }
    }

    fn ai_ready(&self) -> bool {
        if self.ai.provider == Provider::Custom && self.ai.custom_url.trim().is_empty() {
            return false;
        }
        !self.ai.provider.requires_key() || self.ai_has_key
    }

    fn fetch_models(&self) -> Task<Message> {
        let endpoint = self.ai.endpoint();
        let provider = endpoint.provider;
        background(move || { ai::list_models(endpoint) }, move |result| Message::AiModelsLoaded(provider, result))
    }

    fn ask_in_settings(&mut self, status: &str) -> Task<Message> {
        let task = if self.show_settings { Task::none() } else { self.update(Message::ToggleSettings) };
        self.status = status.to_owned();
        task
    }

    fn start_suggestion(&mut self, rejected: Vec<String>) -> Task<Message> {
        if !self.ai_ready() {
            let prompt = match self.ai.provider {
                Provider::Custom => "Add your server's address to get suggestions.".to_owned(),
                provider => format!("Add your {} API key to get suggestions.", provider.name()),
            };
            return self.ask_in_settings(&prompt);
        }
        if self.ai.model.trim().is_empty() {
            return self.ask_in_settings("Choose a model to get suggestions.");
        }
        if self.ai.instructions().is_none() {
            return self.ask_in_settings("Write your Suggestion Instructions, or choose another preset.");
        }
        let Some((tab, _, hunk)) = self.selection() else {
            return Task::none();
        };
        let key = hunk.key;
        let retrying = !rejected.is_empty();
        let joining = !retrying && matches!(self.ahead.get(&key), Some(AheadState::Loading));
        let gate = self.suggestion_gate(tab, hunk, retrying);
        let path = tab.path.clone();
        let settings = self.suggestion_settings(tab, key);
        let prepared = self.suggestion_request(tab, hunk, rejected.clone());
        self.reword = None;
        if joining {
            self.suggestion = Some(Suggestion { key, state: SuggestionState::Loading, rejected });
            return Task::none();
        }
        match gate {
            SuggestGate::Cached(reply) => {
                self.suggestion = Some(Suggestion { key, state: SuggestionState::Ready(reply), rejected });
                return Task::none();
            }
            SuggestGate::Wait(seconds) => {
                let wait = format!("Wait {seconds} s before asking again for this comment.");
                self.suggestion = Some(Suggestion { key, state: SuggestionState::Failed(wait), rejected });
                return Task::none();
            }
            SuggestGate::Ask => {}
        }
        let Some((highlighter, request)) = prepared else {
            return Task::none();
        };
        self.suggest_cache.insert(
            (path.clone(), key),
            CachedSuggestion { asked: Instant::now(), settings, reply: None },
        );
        self.suggestion = Some(Suggestion { key, state: SuggestionState::Loading, rejected });
        self.suggest_seq += 1;
        let seq = self.suggest_seq;
        background(
            move || { ai::suggest(&highlighter, request) },
            move |result| Message::SuggestionReady(seq, path, key, result),
        )
    }

    fn can_undo(&self) -> bool {
        !self.busy && self.tab().is_some_and(|t| !t.undo.is_empty())
    }

    fn can_fix_all(&self) -> bool {
        !self.busy
            && self.tab().is_some_and(|t| t.read_only.is_none())
            && self.fix_count() > 0
    }

    fn next_from(&self, position: (usize, usize)) -> Option<usize> {
        let tab = self.tab()?;
        tab.doc
            .hunks
            .iter()
            .position(|h| (h.first, h.start) >= position && !tab.dimmed.contains(&h.key))
    }

    fn advance_from(&mut self, position: (usize, usize)) -> Task<Message> {
        let Some(tab) = self.tab_mut() else {
            return Task::none();
        };
        let Some(count) = &mut tab.review else {
            tab.selected = None;
            return Task::none();
        };
        *count += 1;
        let reviewed = *count;
        match self.next_from(position) {
            Some(next) => self.jump_to(next),
            None => match self.review_next_file() {
                Some(task) => task,
                None => {
                    self.finish_review(format!("Done: went through {reviewed} comments."));
                    Task::none()
                }
            },
        }
    }

    fn finish_review(&mut self, status: String) {
        self.select(None);
        if let Some(tab) = self.tab_mut() {
            tab.review = None;
        }
        self.forget_ahead();
        self.status = status;
    }

    fn run_action(&mut self, action: Action) -> Task<Message> {
        let available = self.available();
        let allowed = match action {
            Action::Fix => available.fix,
            Action::Ignore => available.ignore,
            Action::Squash => available.squash,
            Action::Delete => available.delete,
            Action::Reword => available.reword,
            Action::Suggest => available.suggest,
        };
        if !allowed {
            return Task::none();
        }
        let (Some(hunk), Some(highlighter)) = (self.selected_hunk(), self.highlighter.clone()) else {
            return Task::none();
        };
        let Some(tab) = self.tab() else {
            return Task::none();
        };
        let syntax = highlighter.syntax_for(&tab.doc.language);
        let decision = tab.selected.and_then(|i| tab.decisions.get(i)).cloned();
        match action {
            Action::Ignore => {
                let line = hunk.first + 1;
                let (Some(index), Some(path)) = (self.active, self.tab().map(|t| t.path.clone())) else {
                    return Task::none();
                };
                if self.tabs[index].ignored.remove(&hunk.key) {
                    if let Some(keys) = self.ignored_store.get_mut(&path) {
                        keys.remove(&hunk.key);
                        if keys.is_empty() {
                            self.ignored_store.remove(&path);
                        }
                    }
                    self.refresh_decisions(index);
                    self.recount();
                    self.save_session();
                    self.status = format!("The comment on line {line} is no longer ignored.");
                    return Task::none();
                }
                let tab = &mut self.tabs[index];
                tab.ignored.insert(hunk.key);
                tab.dimmed.insert(hunk.key);
                self.ignored_store.entry(path).or_default().insert(hunk.key);
                self.recount();
                self.save_session();
                self.status = format!("Ignored the comment on line {line}.");
                self.advance_from((hunk.last, hunk.end))
            }
            Action::Suggest => self.start_suggestion(Vec::new()),
            Action::Reword => {
                self.suggestion = None;
                self.reword = Some(text_editor::Content::with_text(&hunk.raw));
                operation::focus(Id::new(REWORD_EDITOR))
            }
            Action::Delete => self.start_changes(vec![(hunk, Change::Delete)]),
            Action::Squash => match cleanup::squash(&hunk, syntax) {
                Some(text) => self.start_changes(vec![(hunk, Change::Replace(text))]),
                None => Task::none(),
            },
            Action::Fix => match decision {
                Some(Decision::Squash(text)) => {
                    self.start_changes(vec![(hunk, Change::Replace(text))])
                }
                Some(Decision::Delete(_)) => self.start_changes(vec![(hunk, Change::Delete)]),
                _ => Task::none(),
            },
        }
    }

    fn start_changes(&mut self, changes: Vec<(Hunk, Change)>) -> Task<Message> {
        let (Some(tab), Some(highlighter)) = (self.tab(), self.highlighter.clone()) else {
            return Task::none();
        };
        let path = tab.path.clone();
        let reply_path = path.clone();
        let hash = tab.hash;
        let doc = tab.doc.clone();
        let theme = self.theme_name.clone();
        self.busy = true;
        background(
            move || { cleanup::apply_changes(&highlighter, &path, &theme, hash, &doc, &changes) },
            move |result| Message::Applied(reply_path.clone(), result),
        )
    }

    /// How many comments in the active file the chosen mode would change.
    fn fix_count(&self) -> usize {
        let Some(tab) = self.tab() else {
            return 0;
        };
        let scope = self.scope_for(&tab.path);
        let syntax = self.highlighter.as_ref().and_then(|h| h.syntax_for(&tab.doc.language));
        tab.doc
            .hunks
            .iter()
            .zip(&tab.decisions)
            .filter(|(hunk, _)| !tab.ignored.contains(&hunk.key) && in_scope(scope, hunk))
            .filter(|(hunk, decision)| self.fix_mode.applies(hunk, decision, syntax))
            .count()
    }

    fn on_key(&mut self, key: Key, modifiers: Modifiers, repeat: bool) -> Task<Message> {
        if self.confirm_exit.is_some() {
            return match key {
                Key::Named(Named::Escape) => self.update(Message::ExitCancelled),
                Key::Named(Named::Enter) => self.update(Message::ExitConfirmed),
                _ => Task::none(),
            };
        }
        if self.whats_new.is_some() {
            return match key {
                Key::Named(Named::Escape | Named::Enter) => self.update(Message::CloseWhatsNew),
                _ => Task::none(),
            };
        }
        if self.palette_open {
            return match key {
                Key::Named(Named::ArrowUp) => self.update(Message::PaletteMove(-1)),
                Key::Named(Named::ArrowDown) => self.update(Message::PaletteMove(1)),
                Key::Named(Named::Escape) => self.update(Message::ClosePalette),
                _ => Task::none(),
            };
        }
        if repeat {
            return Task::none();
        }
        if self.plan.is_some() && key != Key::Named(Named::Escape) {
            return Task::none();
        }
        let typing_elsewhere = self.reword.is_some() || self.show_settings;
        let letter = match key.as_ref() {
            Key::Character(c) => c.to_lowercase(),
            _ => String::new(),
        };

        if modifiers.control() {
            let message = match (letter.as_str(), modifiers.shift()) {
                ("o", false) => Message::PickFile,
                ("o", true) => Message::PickFolder,
                ("w", false) => Message::CloseFile,
                ("b", false) if self.root.is_some() => Message::ToggleSidebar,
                (",", false) => Message::ToggleSettings,
                ("f", false) if self.tab().is_some() && !typing_elsewhere => Message::OpenFind,
                ("f", true) => Message::OpenSearch,
                ("p", true) => Message::OpenPalette,
                ("s", false) if self.show_settings && self.can_save_rules() => Message::SaveRules,
                ("z", false) if self.can_undo() && !typing_elsewhere => Message::Undo,
                _ => return Task::none(),
            };
            return self.update(message);
        }

        if key == Key::Named(Named::Escape) {
            let reviewing = self.tab().is_some_and(|t| t.review.is_some());
            let selected = self.tab().is_some_and(|t| t.selected.is_some());
            let message = if self.menu.is_some() {
                Message::CloseMenu
            } else if self.plan.is_some() {
                Message::PlanCancel
            } else if self.reword.is_some() {
                Message::RewordCancel
            } else if self.show_settings {
                Message::ToggleSettings
            } else if self.find_open {
                Message::CloseFind
            } else if self.suggestion.is_some() {
                Message::SuggestionReject
            } else if reviewing {
                Message::StopReview
            } else if selected {
                Message::Deselect
            } else {
                return Task::none();
            };
            return self.update(message);
        }

        if key == Key::Named(Named::F3) && self.find_open {
            return self.update(Message::FindStep(!modifiers.shift()));
        }

        if modifiers.alt() || modifiers.logo() || modifiers.shift() {
            return Task::none();
        }
        if typing_elsewhere || self.menu.is_some() {
            return Task::none();
        }
        if key == Key::Named(Named::Delete) {
            return self.run_action(Action::Delete);
        }
        let reviewing = self.tab().is_some_and(|t| t.review.is_some());
        if let Some(suggestion) = &self.suggestion {
            let ready = matches!(suggestion.state, SuggestionState::Ready(_));
            if key == Key::Named(Named::Enter) && ready {
                return self.update(Message::SuggestionApprove);
            }
            if letter == "r" {
                return self.update(Message::SuggestionRetry);
            }
        }
        match letter.as_str() {
            "a" => self.run_action(Action::Suggest),
            "d" if self.selection().is_some() => self.update(Message::ToggleIncludeFunction),
            "f" => self.run_action(Action::Fix),
            "i" => self.run_action(Action::Ignore),
            "s" => self.run_action(Action::Squash),
            "r" => self.run_action(Action::Reword),
            "n" if !self.busy => self.update(Message::Next),
            "p" if !self.busy => self.update(Message::Previous),
            "g" if !reviewing && self.tab().is_some() => self.update(Message::GoThroughAll),
            _ => Task::none(),
        }
    }

    fn adopt(&mut self, index: usize, loaded: Loaded) {
        let generation = self.generation();
        let tab = &mut self.tabs[index];
        let selected_key = tab.selected.and_then(|i| tab.doc.hunks.get(i)).map(|h| h.key);
        tab.doc = loaded.doc;
        tab.hash = loaded.hash;
        tab.read_only = loaded.read_only;
        tab.generation = generation;
        tab.selected = selected_key.and_then(|key| tab.doc.hunks.iter().position(|h| h.key == key));
        if tab.selected.is_none() && self.active == Some(index) {
            self.reword = None;
            self.suggestion = None;
        }
        self.refresh_decisions(index);
    }

    fn restore_code_scroll(&self) -> Task<Message> {
        match self.tab().map(|tab| tab.offset) {
            Some(offset) => scroll_code(offset.x, offset.y),
            None => Task::none(),
        }
    }

    fn rescan_changed(&mut self, paths: Vec<PathBuf>) -> Task<Message> {
        let (Some(root), Some(highlighter)) = (self.root.clone(), self.highlighter.clone()) else {
            return Task::none();
        };
        let rules_changed = paths.iter().any(|path| path.ends_with(".gitignore"));
        if rules_changed || paths.len() > MAX_INCREMENTAL_RESCAN {
            return self.start_scan();
        }
        let generation = self.scan_generation;
        let rules = self.rules.clone();
        let theme = self.theme_name.clone();
        background(
            move || { scan::rescan(&highlighter, &rules, &root, &paths, &theme) },
            move |(updated, removed)| Message::ScanUpdated(generation, updated, removed),
        )
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::HighlighterReady(Ok(highlighter)) => {
                let saved = rules::app_settings_path()
                    .and_then(|path| rules::read_settings(&path))
                    .and_then(|settings| settings.theme);
                let wanted = self.options.theme.clone().or(saved);
                let name = wanted
                    .filter(|t| highlighter.palettes.contains_key(&t.to_lowercase()))
                    .or_else(|| {
                        ["Dracula Theme", "Dracula", "Dark+"]
                            .into_iter()
                            .find(|t| highlighter.palettes.contains_key(&t.to_lowercase()))
                            .map(str::to_owned)
                    })
                    .or_else(|| highlighter.theme_names.first().cloned())
                    .unwrap_or_default();
                let saved_icons = rules::app_settings_path()
                    .and_then(|path| rules::read_settings(&path))
                    .and_then(|settings| settings.icon_theme);
                let icon_entry = highlighter
                    .icon_themes
                    .iter()
                    .find(|entry| Some(&entry.label) == saved_icons.as_ref())
                    .cloned();
                self.icon_theme_names = icons::names(&highlighter.icon_themes);
                self.highlighter = Some(highlighter);
                self.apply_theme(name);
                self.apply_rules();
                self.rebuild_tree();
                self.status = "Ready.".into();
                let mut tasks: Vec<_> =
                    (0..self.tabs.len()).map(|i| self.load_tab(i, None)).collect();
                if let Some(open) = self.options.open.take() {
                    tasks.push(self.open_file(open));
                }
                if let Some(entry) = icon_entry {
                    self.icon_theme_name = entry.label.clone();
                    tasks.push(self.load_icon_theme(entry));
                }
                if let Some(session) = self.restore.take() {
                    for path in session.tabs.into_iter().filter(|path| path.is_file()) {
                        tasks.push(self.open_file(path));
                    }
                    if let Some(index) = session.active.filter(|index| *index < self.tabs.len()) {
                        tasks.push(self.activate(index));
                    }
                }
                self.save_session();
                tasks.push(self.start_scan());
                tasks.push(self.update(Message::AutoCheckForUpdates));
                Task::batch(tasks)
            }
            Message::HighlighterReady(Err(error)) => {
                self.status = format!("Couldn't load the grammars: {error}");
                Task::none()
            }
            Message::PickFolder => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_folder()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                },
                Message::FolderPicked,
            ),
            Message::FolderPicked(Some(folder)) => {
                self.set_root(folder);
                self.start_scan()
            }
            Message::FolderPicked(None) => Task::none(),
            Message::PickFile => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_file()
                        .await
                        .map(|handle| handle.path().to_path_buf())
                },
                Message::FilePicked,
            ),
            Message::FilePicked(Some(path)) => self.open_file(path),
            Message::FilePicked(None) => Task::none(),
            Message::CloseFile => match self.active {
                Some(index) => self.update(Message::CloseTab(index)),
                None => Task::none(),
            },
            Message::CloseFolder => {
                let Some(root) = self.root.take() else {
                    return Task::none();
                };
                self.forget_folder_state();
                self.side_panel = SidePanel::Files;
                self.expanded.clear();
                self.entries.clear();
                self.entry_icons.clear();
                self.tabs.retain(|tab| !tab.path.starts_with(&root));
                self.reword = None;
                self.stop_scan();
                self.load_settings();
                self.active = self.tabs.len().checked_sub(1);
                self.save_session();
                match self.active {
                    Some(index) => self.activate(index),
                    None => Task::none(),
                }
            }
            Message::Exit => {
                self.menu = None;
                let concerns = self.exit_concerns();
                if concerns.is_empty() {
                    self.quit()
                } else {
                    self.confirm_exit = Some(concerns);
                    operation::focus(Id::new(NO_WIDGET))
                }
            }
            Message::ExitConfirmed => {
                self.confirm_exit = None;
                self.quit()
            }
            Message::ExitCancelled => {
                self.confirm_exit = None;
                self.update_restart = false;
                if self.reword.is_some() { operation::focus(Id::new(REWORD_EDITOR)) } else { Task::none() }
            }
            Message::Quit(handle) => {
                self.placement = window_state::read(handle).or(self.placement);
                if let UpdateState::Ready { version, notes, .. } = &self.update {
                    self.update_notes = Some((version.clone(), notes.clone()));
                }
                self.save_session();
                if let UpdateState::Ready { path, .. } = &self.update {
                    let installed = update::install(path);
                    let restart = std::mem::take(&mut self.update_restart);
                    match installed.and_then(|()| if restart { update::relaunch() } else { Ok(()) }) {
                        Ok(()) => {}
                        Err(error) if restart => {
                            self.status = error;
                            return Task::none();
                        }
                        Err(_) => {}
                    }
                }
                iced::exit()
            }
            Message::CheckForUpdates => self.check_for_updates(true),
            Message::AutoCheckForUpdates => {
                if self.auto_update { self.check_for_updates(false) } else { Task::none() }
            }
            Message::UpdateChecked(manual, result) => {
                match result {
                    Ok(Some(release)) if self.auto_update => return self.start_download(release),
                    Ok(Some(release)) => self.update = UpdateState::Available(release),
                    Ok(None) if manual => self.update = UpdateState::UpToDate,
                    Err(error) if manual => self.update = UpdateState::Failed(error),
                    _ => self.update = UpdateState::Idle,
                }
                Task::none()
            }
            Message::UpdateDownload(release) => self.start_download(release),
            Message::UpdateProgress(update::Progress::Bytes(got, total)) => {
                if let UpdateState::Downloading { percent, .. } = &mut self.update {
                    *percent = got * 100 / total.max(1);
                }
                Task::none()
            }
            Message::UpdateProgress(update::Progress::Done(result)) => {
                self.update_abort = None;
                self.update = match (result, std::mem::replace(&mut self.update, UpdateState::Idle)) {
                    (Ok(path), UpdateState::Downloading { version, notes, .. }) => {
                        self.status = format!("Version {version} is ready. It installs when you close the app.");
                        UpdateState::Ready { version, path, notes }
                    }
                    (Err(error), _) => UpdateState::Failed(error),
                    (_, other) => other,
                };
                Task::none()
            }
            Message::UpdateRestart => {
                self.update_restart = true;
                self.update(Message::Exit)
            }
            Message::SetAutoUpdate(value) => {
                self.auto_update = value;
                self.save_session();
                if value { self.check_for_updates(false) } else { Task::none() }
            }
            Message::FixModePicked(mode) => {
                self.fix_mode = mode;
                Task::none()
            }
            Message::CloseWhatsNew => {
                self.whats_new = None;
                Task::none()
            }
            Message::AiSuggestAhead(value) => {
                self.ai.suggest_ahead = value;
                self.save_ai();
                Task::none()
            }
            Message::SpinnerFrame => Task::none(),
            Message::SuggestedAhead(epoch, path, key, result) => {
                self.remember_reply(path, key, &result);
                if epoch != self.ahead_epoch {
                    return Task::none();
                }
                if result.as_ref().is_err_and(|error| error.contains("rate limit")) {
                    self.ahead_paused = Some(Instant::now() + AHEAD_PAUSE);
                }
                let state = match result {
                    Ok(suggested) => AheadState::Ready(suggested),
                    Err(error) => AheadState::Failed(error),
                };
                if let Some(shown) = self.suggestion.as_mut().filter(|shown| {
                    shown.key == key && shown.rejected.is_empty() && matches!(shown.state, SuggestionState::Loading)
                }) {
                    shown.state = match &state {
                        AheadState::Ready(suggested) => SuggestionState::Ready(suggested.clone()),
                        AheadState::Failed(error) => SuggestionState::Failed(error.clone()),
                        AheadState::Loading => SuggestionState::Loading,
                    };
                }
                self.ahead.insert(key, state);
                self.top_up_ahead()
            }
            Message::AiProviderPicked(provider) => {
                if provider == self.ai.provider {
                    return Task::none();
                }
                self.ai.use_provider(provider);
                self.ai_has_key = ai::has_key(provider);
                self.ai_key_input.clear();
                self.ai_models.clear();
                self.ai_models_error = None;
                self.save_ai();
                if self.ai_ready() { self.fetch_models() } else { Task::none() }
            }
            Message::AiCustomUrlInput(value) => {
                self.ai.custom_url = value;
                self.ai_models.clear();
                self.ai_models_error = None;
                self.save_ai();
                Task::none()
            }
            Message::AiCustomUrlSubmit => {
                if self.ai_ready() { self.fetch_models() } else { Task::none() }
            }
            Message::OpenPalette => {
                self.palette_open = true;
                self.palette_query.clear();
                self.palette_selected = 0;
                operation::focus(Id::new(PALETTE_INPUT))
            }
            Message::ClosePalette => {
                self.palette_open = false;
                Task::none()
            }
            Message::PaletteInput(value) => {
                self.palette_query = value;
                self.palette_selected = 0;
                Task::none()
            }
            Message::PaletteMove(step) => {
                let count = self.palette_matches().len() as i32;
                if count > 0 {
                    self.palette_selected = (self.palette_selected as i32 + step).rem_euclid(count) as usize;
                }
                Task::none()
            }
            Message::PaletteRun(index) => {
                let chosen = self.palette_matches().into_iter().nth(index);
                self.palette_open = false;
                match chosen {
                    Some((.., message)) => self.update(message),
                    None => Task::none(),
                }
            }
            Message::OpenMenu(menu) => {
                self.menu = if self.menu == Some(menu) { None } else { Some(menu) };
                Task::none()
            }
            Message::HoverMenu(menu) => {
                if self.menu.is_some() {
                    self.menu = Some(menu);
                }
                Task::none()
            }
            Message::CloseMenu => {
                self.menu = None;
                Task::none()
            }
            Message::MenuItem(message) => {
                self.menu = None;
                self.update(*message)
            }
            Message::TitleBarPressed => {
                let now = Instant::now();
                let double = self.last_title_press.is_some_and(|last| now - last < DOUBLE_CLICK);
                self.last_title_press = (!double).then_some(now);
                self.title_armed = !double;
                if double {
                    self.update(Message::WindowToggleMaximize)
                } else {
                    Task::none()
                }
            }
            Message::TitleBarReleased => {
                self.title_armed = false;
                Task::none()
            }
            Message::TitleBarDragged => {
                self.title_armed = false;
                self.last_title_press = None;
                window::latest().and_then(window::drag)
            }
            Message::WindowMinimize => {
                window::latest().and_then(|id| window::minimize(id, true))
            }
            Message::WindowToggleMaximize => window::latest().and_then(window::toggle_maximize),
            Message::WindowResize(edge) => {
                window::latest().and_then(move |id| window::drag_resize(id, edge))
            }
            Message::WindowResized => window::latest()
                .and_then(window::is_maximized)
                .map(Message::MaximizedChanged),
            Message::MaximizedChanged(maximized) => {
                self.maximized = maximized;
                Task::none()
            }
            Message::ToggleSidebar => {
                self.show_sidebar = !self.show_sidebar;
                Task::none()
            }
            Message::KeyPressed(key, modifiers, repeat) => self.on_key(key, modifiers, repeat),
            Message::ModifiersChanged(modifiers) => {
                self.ctrl_held = modifiers.control();
                self.shift_held = modifiers.shift();
                Task::none()
            }
            Message::SaveRules => {
                if self.can_save_rules() { self.save_rules() } else { Task::none() }
            }
            Message::DiscardRules => {
                self.settings = self.saved_settings.clone();
                self.reset_rule_inputs();
                self.status = "Rule changes discarded.".into();
                self.rules_edited()
            }
            Message::ImpactReady(seq, impact) => {
                if seq == self.impact_seq.load(Ordering::SeqCst) && impact.is_some() {
                    self.rule_impact = impact;
                }
                Task::none()
            }
            Message::RuleSampleEdited(action) => {
                let edited = action.is_edit();
                self.rule_sample.perform(action);
                if edited {
                    self.update_rule_sample();
                }
                Task::none()
            }
            Message::GitRead(root, repo) => {
                self.git_running = false;
                let again = std::mem::take(&mut self.git_again);
                if self.root.as_ref() == Some(&root) && repo != self.repo {
                    self.repo_dirs.clear();
                    for path in repo.iter().flat_map(|repo| repo.files.keys()) {
                        self.repo_dirs.extend(
                            path.ancestors().skip(1).take_while(|dir| dir.starts_with(&root)).map(Path::to_path_buf),
                        );
                    }
                    self.repo = repo;
                    if self.only_changed {
                        for index in 0..self.tabs.len() {
                            self.refresh_decisions(index);
                        }
                        self.recount();
                        self.rebuild_plan();
                    }
                }
                if again { self.refresh_git() } else { Task::none() }
            }
            Message::WindowFocused => self.refresh_git(),
            Message::ToggleOnlyChanged => {
                self.only_changed = !self.only_changed;
                for index in 0..self.tabs.len() {
                    self.refresh_decisions(index);
                }
                self.recount();
                self.rebuild_plan();
                self.save_session();
                self.status = if self.only_changed {
                    "Cleaning only comments on lines changed since the last commit.".into()
                } else {
                    "Cleaning every comment.".into()
                };
                Task::none()
            }
            Message::OpenFind => {
                self.find_open = true;
                let input = Id::new(FIND_INPUT);
                let focus = operation::focus(input.clone()).chain(operation::select_all(input));
                Task::batch([focus, self.run_find(true)])
            }
            Message::CloseFind => {
                self.find_open = false;
                self.find_goto = None;
                self.run_find(false)
            }
            Message::FindInput(value) => {
                self.find_query.text = value;
                self.run_find(true)
            }
            Message::FindToggle(option) => {
                option.flip(&mut self.find_query);
                self.run_find(true)
            }
            Message::FindSubmit => self.update(Message::FindStep(!self.shift_held)),
            Message::FindStep(forward) => {
                let count = self.find_matches.len();
                if count == 0 {
                    return Task::none();
                }
                let next = match (self.find_current, forward) {
                    (None, _) => 0,
                    (Some(current), true) => (current + 1) % count,
                    (Some(current), false) => (current + count - 1) % count,
                };
                self.find_current = Some(next);
                self.reveal_match(next)
            }
            Message::FindResults(seq, matches) => {
                if seq != self.find_seq {
                    return Task::none();
                }
                let reveal = std::mem::take(&mut self.find_reveal);
                let loaded = self.tab().is_some_and(|tab| !tab.doc.lines.is_empty());
                let goto = self
                    .find_goto
                    .clone()
                    .filter(|(path, ..)| loaded && self.tab().is_some_and(|tab| tab.path == *path));
                let previous = self
                    .find_current
                    .and_then(|index| self.find_matches.get(index))
                    .map(|found| (found.line, found.start));
                let top = self.tab().map_or(0, |tab| (tab.offset.y / LINE_HEIGHT) as u32);
                let target = match &goto {
                    Some((_, line, column)) => (*line, *column),
                    None if reveal => (top, 0),
                    None => previous.unwrap_or((top, 0)),
                };
                self.find_matches = matches;
                let count = self.find_matches.len();
                let after = self.find_matches.partition_point(|m| (m.line, m.start) < target);
                self.find_current = match (count, after == count) {
                    (0, _) => None,
                    (_, true) if goto.is_some() || !reveal => Some(count - 1),
                    (_, true) => Some(0),
                    (_, false) => Some(after),
                };
                if let Some((_, line, _)) = &goto {
                    self.find_goto = None;
                    if count == 0 {
                        return scroll_code(0.0, (*line as f32 * LINE_HEIGHT - self.code_view.height / 2.0).max(0.0));
                    }
                }
                match self.find_current {
                    Some(current) if goto.is_some() || reveal => self.reveal_match(current),
                    _ => Task::none(),
                }
            }
            Message::EscapeInTextBox => {
                if self.confirm_exit.is_some() {
                    return self.update(Message::ExitCancelled);
                }
                if self.palette_open {
                    return self.update(Message::ClosePalette);
                }
                if self.show_settings || self.reword.is_some() {
                    return Task::none();
                }
                if self.find_open {
                    return self.update(Message::CloseFind);
                }
                if self.side_panel == SidePanel::Search {
                    self.side_panel = SidePanel::Files;
                }
                Task::none()
            }
            Message::OpenSearch => {
                if self.root.is_none() {
                    self.status = "Open a folder to search all of its files.".into();
                    return Task::none();
                }
                self.side_panel = SidePanel::Search;
                self.show_sidebar = true;
                let input = Id::new(SEARCH_INPUT);
                operation::focus(input.clone()).chain(operation::select_all(input))
            }
            Message::ToggleSearchPanel => {
                if self.side_panel == SidePanel::Search {
                    self.side_panel = SidePanel::Files;
                    Task::none()
                } else {
                    self.update(Message::OpenSearch)
                }
            }
            Message::SearchInput(value) => {
                self.search_query.text = value;
                Task::batch([self.search_to_top(), self.start_search()])
            }
            Message::SearchToggle(option) => {
                option.flip(&mut self.search_query);
                Task::batch([self.search_to_top(), self.start_search()])
            }
            Message::SearchBatch(seq, batch) => {
                if seq == self.search_seq && !batch.is_empty() {
                    if std::mem::take(&mut self.search_replacing) {
                        self.search_files.clear();
                    }
                    self.search_files.extend(batch);
                    self.search_files.sort_by_cached_key(|file| file.label.to_lowercase());
                    self.rebuild_search_rows();
                }
                Task::none()
            }
            Message::SearchDone(seq) => {
                if seq == self.search_seq {
                    self.search_running = false;
                    self.search_abort = None;
                    if std::mem::take(&mut self.search_replacing) {
                        self.search_files.clear();
                        self.rebuild_search_rows();
                    }
                }
                Task::none()
            }
            Message::SearchScrolled(viewport) => {
                self.search_scroll = (viewport.absolute_offset().y, viewport.bounds().height);
                Task::none()
            }
            Message::SearchToggleFile(path) => {
                if !self.search_collapsed.remove(&path) {
                    self.search_collapsed.insert(path);
                }
                self.rebuild_search_rows();
                Task::none()
            }
            Message::SearchOpenHit(file, hit) => {
                let Some((path, line, start)) = self
                    .search_files
                    .get(file)
                    .and_then(|found| Some((found.path.clone(), found.hits.get(hit)?.line, found.hits.get(hit)?.start)))
                else {
                    return Task::none();
                };
                self.find_query = self.search_query.clone();
                self.find_open = true;
                self.find_goto = Some((path.clone(), line, start));
                self.open_file(path)
            }
            Message::CodeClicked => {
                let reviewing = self.tab().is_some_and(|t| t.review.is_some());
                if !reviewing && self.reword.is_none() {
                    self.select(None);
                }
                Task::none()
            }
            Message::IconThemePicked(name) => {
                if let Err(error) = rules::save_icon_theme(&name) {
                    self.status = error;
                }
                self.settings.icon_theme = Some(name.clone());
                self.saved_settings.icon_theme = Some(name.clone());
                self.icon_theme_name = name.clone();
                let entry = self.highlighter.as_ref().and_then(|highlighter| {
                    highlighter.icon_themes.iter().find(|e| e.label == name).cloned()
                });
                match entry {
                    Some(entry) => self.load_icon_theme(entry),
                    None => {
                        self.icon_theme = None;
                        self.refresh_icons();
                        Task::none()
                    }
                }
            }
            Message::IconsLoaded(label, result) => {
                if label != self.icon_theme_name {
                    return Task::none();
                }
                match result {
                    Ok(loaded) => match loaded.font.clone() {
                        Some(font) => iced::font::load(font).map(move |_| {
                            Message::IconFontReady(label.clone(), loaded.clone())
                        }),
                        None => self.update(Message::IconFontReady(label, loaded)),
                    },
                    Err(error) => {
                        self.status = format!("Couldn't load the icon theme: {error}");
                        Task::none()
                    }
                }
            }
            Message::IconFontReady(label, loaded) => {
                if label == self.icon_theme_name {
                    self.icon_theme = Some(loaded);
                    self.refresh_icons();
                }
                Task::none()
            }
            Message::EntryPressed(index) => {
                let Some(entry) = self.entries.get(index).cloned() else {
                    return Task::none();
                };
                if entry.is_dir {
                    if !self.expanded.remove(&entry.path) {
                        self.expanded.insert(entry.path);
                    }
                    self.rebuild_tree();
                    Task::none()
                } else {
                    self.open_file(entry.path)
                }
            }
            Message::ActivateTab(index) => {
                let task = self.activate(index);
                self.save_session();
                task
            }
            Message::CloseTab(index) => {
                self.tabs.remove(index);
                self.reword = None;
                self.active = match self.active {
                    _ if self.tabs.is_empty() => None,
                    Some(active) if active > index => Some(active - 1),
                    Some(active) if active == index => Some(index.min(self.tabs.len() - 1)),
                    other => other,
                };
                self.save_session();
                match self.active {
                    Some(active) => self.activate(active),
                    None => Task::none(),
                }
            }
            Message::Loaded(path, seq, result) => {
                let Some(index) = self.tabs.iter().position(|t| t.path == path) else {
                    return Task::none();
                };
                if self.tabs[index].load_seq != seq {
                    return Task::none();
                }
                match result {
                    Ok(Some(loaded)) => {
                        let lines = loaded.doc.lines.len();
                        println!("loaded {}: {lines} lines in {:?}", path.display(), loaded.doc.tokenize_time);
                        self.adopt(index, loaded);
                        let find = if self.active == Some(index) { self.run_find(false) } else { Task::none() };
                        if self.pending_review.as_ref() == Some(&path) {
                            self.pending_review = None;
                            return Task::batch([find, self.update(Message::GoThroughAll)]);
                        }
                        if self.options.autoscroll && self.auto.is_none() {
                            self.options.autoscroll = false;
                            viewer::RECORD.store(true, Ordering::Relaxed);
                            self.auto = Some(Auto {
                                y: 0.0,
                                frames: 0,
                                last: None,
                                intervals: Vec::with_capacity(AUTOSCROLL_FRAMES),
                            });
                            return scroll_code(0.0, 1.0);
                        }
                        return find;
                    }
                    Ok(None) => {}
                    Err(error) => self.status = error,
                }
                Task::none()
            }
            Message::FilesChanged(paths) => {
                let (git_paths, paths): (Vec<PathBuf>, Vec<PathBuf>) =
                    paths.into_iter().partition(|path| is_git_internal(self.root.as_deref(), path));
                let repo_moved = !paths.is_empty() || git_paths.iter().any(|path| is_git_state(path));
                let git_task = if repo_moved { self.refresh_git() } else { Task::none() };
                if paths.is_empty() {
                    return git_task;
                }
                let search_stale = self.side_panel == SidePanel::Search
                    && !self.search_running
                    && !self.search_query.text.is_empty()
                    && paths.iter().any(|path| {
                        self.scan.contains_key(path) || self.search_files.iter().any(|file| file.path == *path)
                    });
                let parent_shown = |p: &PathBuf| {
                    p.parent().is_some_and(|parent| {
                        self.root.as_deref() == Some(parent) || self.expanded.contains(parent)
                    })
                };
                if paths.iter().any(parent_shown) {
                    self.rebuild_tree();
                }
                let reloads: Vec<(usize, u64)> = self
                    .tabs
                    .iter()
                    .enumerate()
                    .filter(|(_, tab)| paths.contains(&tab.path))
                    .map(|(index, tab)| (index, tab.hash))
                    .collect();
                let mut tasks: Vec<_> =
                    reloads.into_iter().map(|(i, hash)| self.load_tab(i, Some(hash))).collect();
                tasks.push(self.rescan_changed(paths));
                tasks.push(git_task);
                if search_stale {
                    tasks.push(self.start_search());
                }
                Task::batch(tasks)
            }
            Message::ScanBatch(generation, batch) => {
                if generation == self.scan_generation {
                    self.scan.extend(batch.into_iter().map(|file| (file.path.clone(), file)));
                    if self.last_recount.elapsed() >= RECOUNT_INTERVAL {
                        self.recount();
                    }
                }
                Task::none()
            }
            Message::ScanDone(generation) => {
                if generation == self.scan_generation {
                    self.scan_running = false;
                    self.scan_abort = None;
                    self.recount();
                    if self.show_settings {
                        return self.test_rules();
                    }
                }
                Task::none()
            }
            Message::ScanUpdated(generation, updated, removed) => {
                if generation == self.scan_generation {
                    for path in removed {
                        self.scan.remove(&path);
                    }
                    self.scan.extend(updated.into_iter().map(|file| (file.path.clone(), file)));
                    self.recount();
                }
                Task::none()
            }
            Message::CleanFolder => {
                self.refresh_scan_decisions();
                let plan = self.folder_plan();
                if plan.files.is_empty() {
                    self.status = if self.only_changed && self.repo.is_some() {
                        "The rules have nothing to change on the lines you've changed.".into()
                    } else {
                        "The rules have nothing to change in this folder.".into()
                    };
                } else {
                    self.plan = Some(plan);
                }
                Task::none()
            }
            Message::PlanCancel => {
                self.plan = None;
                self.restore_code_scroll()
            }
            Message::PlanToggleKind(squash, value) => {
                if let Some(plan) = &mut self.plan {
                    for item in plan.files.iter_mut().flat_map(|file| &mut file.items) {
                        if item.is_squash() == squash {
                            item.include = value;
                        }
                    }
                }
                Task::none()
            }
            Message::PlanToggleFile(file, value) => {
                if let Some(file) = self.plan.as_mut().and_then(|plan| plan.files.get_mut(file)) {
                    for item in &mut file.items {
                        item.include = value;
                    }
                }
                Task::none()
            }
            Message::PlanToggleItem(file, item, value) => {
                if let Some(item) = self
                    .plan
                    .as_mut()
                    .and_then(|plan| plan.files.get_mut(file))
                    .and_then(|file| file.items.get_mut(item))
                {
                    item.include = value;
                }
                Task::none()
            }
            Message::PlanExpand(file) => {
                if let Some(file) = self.plan.as_mut().and_then(|plan| plan.files.get_mut(file)) {
                    file.expanded = !file.expanded;
                }
                Task::none()
            }
            Message::PlanApply => {
                let Some(plan) = self.plan.take() else {
                    return Task::none();
                };
                let folder = plan.folder;
                let mut files: Vec<PlanFileChanges> = plan
                    .files
                    .into_iter()
                    .map(|file| PlanFileChanges {
                        path: file.path,
                        hash: file.hash,
                        changes: file
                            .items
                            .into_iter()
                            .filter(|item| item.include)
                            .map(|item| (item.hunk, item.change))
                            .collect(),
                    })
                    .filter(|file| !file.changes.is_empty())
                    .collect();
                if !folder {
                    let scroll = self.restore_code_scroll();
                    return match files.pop() {
                        Some(file) => scroll.chain(self.start_changes(file.changes)),
                        None => scroll,
                    };
                }
                let Some(highlighter) = self.highlighter.clone() else {
                    return Task::none();
                };
                let theme = self.theme_name.clone();
                self.busy = true;
                self.status = "Cleaning the folder…".into();
                let scroll = self.restore_code_scroll();
                scroll.chain(background(
                    move || { scan::apply_plan(&highlighter, &theme, files) },
                    Message::PlanApplied,
                ))
            }
            Message::PlanApplied(outcome) => {
                self.busy = false;
                let PlanOutcome { undo, comments, skipped } = outcome;
                let cleaned = undo.len();
                if !undo.is_empty() {
                    self.folder_undo.push(undo);
                    if self.folder_undo.len() > MAX_FOLDER_UNDO {
                        self.folder_undo.remove(0);
                    }
                }
                self.status = match skipped.first() {
                    Some(first) => format!(
                        "Cleaned {comments} comments in {cleaned} files. Skipped {}, such as {first}.",
                        skipped.len()
                    ),
                    None => format!("Cleaned {comments} comments in {cleaned} files. Edit > Undo Folder Cleanup restores them."),
                };
                Task::none()
            }
            Message::UndoFolder => {
                let (Some(undo), Some(highlighter)) = (self.folder_undo.pop(), self.highlighter.clone()) else {
                    return Task::none();
                };
                let theme = self.theme_name.clone();
                self.busy = true;
                background(
                    move || { scan::undo_plan(&highlighter, &theme, &undo) },
                    |(restored, left)| Message::FolderUndone(restored, left),
                )
            }
            Message::FolderUndone(restored, left) => {
                self.busy = false;
                self.status = if left == 0 {
                    format!("Restored {restored} files.")
                } else {
                    format!("Restored {restored} files. Left {left} alone because they've changed since.")
                };
                Task::none()
            }
            Message::ThemePicked(name) => {
                if let Err(error) = rules::save_theme(&name) {
                    self.status = error;
                }
                self.settings.theme = Some(name.clone());
                self.saved_settings.theme = Some(name.clone());
                self.apply_theme(name);
                let mut tasks: Vec<_> =
                    (0..self.tabs.len()).map(|i| self.load_tab(i, None)).collect();
                if self.icon_theme.as_ref().is_some_and(|icons| icons.uses_text_color) {
                    let entry = self.highlighter.as_ref().and_then(|highlighter| {
                        highlighter
                            .icon_themes
                            .iter()
                            .find(|e| e.label == self.icon_theme_name)
                            .cloned()
                    });
                    tasks.extend(entry.map(|entry| self.load_icon_theme(entry)));
                }
                Task::batch(tasks)
            }
            Message::Scrolled(viewport) => {
                self.code_view = viewport.bounds().size();
                if let Some(tab) = self.tab_mut() {
                    tab.offset = viewport.absolute_offset();
                }
                Task::none()
            }
            Message::HunkClicked(id) => {
                self.select(Some(id as usize));
                Task::none()
            }
            Message::Do(action) => self.run_action(action),
            Message::Applied(path, result) => {
                self.busy = false;
                let index = self.tabs.iter().position(|t| t.path == path);
                match (result, index) {
                    (Ok(applied), Some(index)) => {
                        let Applied { loaded, previous, resume, changed } = applied;
                        self.tabs[index].load_seq += 1;
                        let after_edit = loaded.hash;
                        self.adopt(index, loaded);
                        let tab = &mut self.tabs[index];
                        tab.undo.push((previous, after_edit));
                        if tab.undo.len() > MAX_UNDO {
                            tab.undo.remove(0);
                        }
                        self.reword = None;
                        self.suggestion = None;
                        self.status = match changed {
                            1 => "Saved. Undo is available.".into(),
                            n => format!("Fixed {n} comments. One Undo restores them all."),
                        };
                        if self.active == Some(index) {
                            let find = self.run_find(false);
                            return Task::batch([find, self.advance_from(resume)]);
                        }
                    }
                    (Err(error), _) => self.status = error,
                    _ => {}
                }
                Task::none()
            }
            Message::RewordEdited(action) => {
                if let Some(content) = &mut self.reword {
                    content.perform(action);
                }
                Task::none()
            }
            Message::RewordApply => {
                let replacement = self.reword.as_ref().map(|c| c.text().trim_end().to_owned());
                match (self.selected_hunk(), replacement) {
                    (Some(hunk), Some(text)) if !text.is_empty() => {
                        self.start_changes(vec![(hunk, Change::Replace(text))])
                    }
                    _ => Task::none(),
                }
            }
            Message::RewordCancel => {
                self.reword = None;
                Task::none()
            }
            Message::SuggestionReady(seq, path, key, result) => {
                self.remember_reply(path, key, &result);
                if seq == self.suggest_seq {
                    if let Some(suggestion) = &mut self.suggestion {
                        suggestion.state = match result {
                            Ok(suggested) => SuggestionState::Ready(suggested),
                            Err(error) => SuggestionState::Failed(error),
                        };
                    }
                }
                Task::none()
            }
            Message::SuggestionApprove => {
                let Some(Suggestion { key, state: SuggestionState::Ready(suggested), .. }) = &self.suggestion else {
                    return Task::none();
                };
                let (key, text) = (*key, suggested.text.clone());
                match self.selected_hunk() {
                    Some(hunk) if hunk.key == key && self.available().reword => {
                        self.start_changes(vec![(hunk, Change::Replace(text))])
                    }
                    _ => Task::none(),
                }
            }
            Message::SuggestionReject => {
                self.suggest_seq += 1;
                let answered = self
                    .suggestion
                    .take()
                    .is_some_and(|shown| !matches!(shown.state, SuggestionState::Loading));
                match self.selected_hunk() {
                    Some(hunk) if answered && self.suggesting_ahead() => self.advance_from((hunk.last, hunk.end)),
                    _ => Task::none(),
                }
            }
            Message::SuggestionRetry => {
                let answered = self.suggestion.as_ref().is_some_and(|shown| !matches!(shown.state, SuggestionState::Loading));
                if answered
                    && let Some((tab, _, hunk)) = self.selection()
                    && let SuggestGate::Wait(seconds) = self.suggestion_gate(tab, hunk, true)
                {
                    self.status = format!("You can retry this comment in {seconds} s.");
                    return Task::none();
                }
                let Some(suggestion) = self.suggestion.take() else {
                    return Task::none();
                };
                let mut rejected = suggestion.rejected;
                match suggestion.state {
                    SuggestionState::Ready(suggested) => rejected.push(suggested.text),
                    SuggestionState::Failed(_) => {}
                    SuggestionState::Loading => {
                        self.suggestion = Some(Suggestion { key: suggestion.key, state: SuggestionState::Loading, rejected });
                        return Task::none();
                    }
                }
                self.start_suggestion(rejected)
            }
            Message::ToggleIncludeFunction => {
                let Some(key) = self.selected_hunk().map(|hunk| hunk.key) else {
                    return Task::none();
                };
                let default = self.ai.include_function;
                self.ahead.remove(&key);
                if let Some(tab) = self.tab_mut() {
                    let current = tab.function_override.get(&key).copied().unwrap_or(default);
                    tab.function_override.insert(key, !current);
                    self.status = format!(
                        "Include function definition is {} for this comment.",
                        if current { "off" } else { "on" }
                    );
                }
                Task::none()
            }
            Message::AiKeyInput(value) => {
                self.ai_key_input = value;
                Task::none()
            }
            Message::AiKeySave => {
                let key = std::mem::take(&mut self.ai_key_input);
                if key.trim().is_empty() {
                    return Task::none();
                }
                match ai::save_key(self.ai.provider, &key) {
                    Ok(()) => {
                        self.ai_has_key = true;
                        self.ai_models.clear();
                        self.ai_models_error = None;
                        self.status = "API key saved and encrypted.".into();
                        self.fetch_models()
                    }
                    Err(error) => {
                        self.status = error;
                        Task::none()
                    }
                }
            }
            Message::AiKeyRemove => {
                match ai::remove_key(self.ai.provider) {
                    Ok(()) => {
                        self.ai_has_key = false;
                        self.ai_models.clear();
                        self.ai_models_error = None;
                        self.status = "API key removed.".into();
                    }
                    Err(error) => self.status = error,
                }
                Task::none()
            }
            Message::AiModelInput(value) => {
                self.ai.model = value;
                self.save_ai();
                Task::none()
            }
            Message::AiModelsLoaded(provider, result) => {
                if provider != self.ai.provider {
                    return Task::none();
                }
                match result {
                    Ok(models) => {
                        self.ai_models = models;
                        self.ai_models_error = None;
                    }
                    Err(error) => self.ai_models_error = Some(error),
                }
                Task::none()
            }
            Message::AiPresetPicked(preset) => {
                self.ai.preset = preset;
                self.save_ai();
                Task::none()
            }
            Message::AiCustomEdited(action) => {
                let edited = action.is_edit();
                self.ai_custom.perform(action);
                if edited {
                    self.ai.custom = self.ai_custom.text();
                    self.save_ai();
                }
                Task::none()
            }
            Message::AiIncludeFunction(value) => {
                self.ai.include_function = value;
                self.save_ai();
                Task::none()
            }
            Message::GoThroughAll => {
                match self.next_from((0, 0)) {
                    Some(first) => {
                        if let Some(tab) = self.tab_mut() {
                            tab.review = Some(0);
                        }
                        self.jump_to(first)
                    }
                    None => {
                        self.status = "No comments left to go through in this file.".into();
                        Task::none()
                    }
                }
            }
            Message::StopReview => {
                self.finish_review("Stopped going through comments.".into());
                Task::none()
            }
            Message::Next => {
                let Some(hunk) = self.selected_hunk() else {
                    return match self.next_from((0, 0)) {
                        Some(first) => self.jump_to(first),
                        None => Task::none(),
                    };
                };
                match self.next_from((hunk.last, hunk.end)) {
                    Some(next) => self.jump_to(next),
                    None if self.tab().is_some_and(|t| t.review.is_some()) => {
                        match self.review_next_file() {
                            Some(task) => task,
                            None => {
                                self.finish_review("Done: reached the last comment.".into());
                                Task::none()
                            }
                        }
                    }
                    None => {
                        self.status = "This is the last comment.".into();
                        Task::none()
                    }
                }
            }
            Message::Previous => {
                let previous = self.tab().and_then(|tab| {
                    let current = tab.selected?;
                    tab.doc.hunks[..current]
                        .iter()
                        .rposition(|h| !tab.dimmed.contains(&h.key))
                });
                match previous {
                    Some(index) => self.jump_to(index),
                    None => {
                        self.status = "This is the first comment.".into();
                        Task::none()
                    }
                }
            }
            Message::Deselect => {
                self.select(None);
                Task::none()
            }
            Message::Undo => {
                let Some(highlighter) = self.highlighter.clone() else {
                    return Task::none();
                };
                let Some(tab) = self.tab_mut() else {
                    return Task::none();
                };
                let Some((previous, hash)) = tab.undo.last().cloned() else {
                    return Task::none();
                };
                let path = tab.path.clone();
                let reply_path = path.clone();
                let theme = self.theme_name.clone();
                self.busy = true;
                background(
                    move || { cleanup::undo(&highlighter, &path, &theme, hash, &previous) },
                    move |result| Message::Undone(reply_path.clone(), result),
                )
            }
            Message::Undone(path, result) => {
                self.busy = false;
                let index = self.tabs.iter().position(|t| t.path == path);
                match (result, index) {
                    (Ok(loaded), Some(index)) => {
                        self.tabs[index].load_seq += 1;
                        self.tabs[index].undo.pop();
                        self.adopt(index, loaded);
                        self.status = "Undone.".into();
                        if self.active == Some(index) {
                            return self.run_find(false);
                        }
                    }
                    (Err(error), _) => self.status = error,
                    _ => {}
                }
                Task::none()
            }
            Message::FixAll => {
                self.select(None);
                if let Some(tab) = self.tab_mut() {
                    tab.review = None;
                }
                self.plan = self.file_plan();
                Task::none()
            }
            Message::ToggleSettings => {
                self.show_settings = !self.show_settings;
                if !self.show_settings {
                    self.refresh_scan_decisions();
                    if self.rules_dirty() {
                        self.status = "Your rule changes aren't saved yet. They'll wait in Settings.".into();
                    }
                    return self.restore_code_scroll();
                }
                let models = self.ai_ready() && self.ai_models.is_empty();
                let models = if models { self.fetch_models() } else { Task::none() };
                self.update_rule_sample();
                Task::batch([models, self.test_rules()])
            }
            Message::SetBool(field, value) => {
                *field(&mut self.settings) = value;
                self.rules_edited()
            }
            Message::SetWidth(value) => {
                self.width_input = value.clone();
                if let Ok(number) = value.parse::<u32>() {
                    self.settings.squash_max_width = number;
                    return self.rules_edited();
                }
                Task::none()
            }
            Message::SetLines(value) => {
                self.lines_input = value.clone();
                if let Ok(number) = value.parse::<u32>() {
                    self.settings.squash_max_lines = number;
                    return self.rules_edited();
                }
                Task::none()
            }
            Message::ProtectEdited(action) => {
                let edited = action.is_edit();
                self.protect_patterns.perform(action);
                if edited {
                    self.settings.protect_patterns = pattern_lines(&self.protect_patterns);
                    return self.rules_edited();
                }
                Task::none()
            }
            Message::DeleteEdited(action) => {
                let edited = action.is_edit();
                self.delete_patterns.perform(action);
                if edited {
                    self.settings.delete_patterns = pattern_lines(&self.delete_patterns);
                    return self.rules_edited();
                }
                Task::none()
            }
            Message::FolderSettings(enabled) => {
                let Some(path) = self.root.as_deref().map(rules::workspace_settings_path) else {
                    return Task::none();
                };
                if enabled {
                    match rules::write_settings(&path, &self.saved_settings) {
                        Ok(()) => self.folder_settings = true,
                        Err(error) => self.status = error,
                    }
                } else {
                    if let Err(error) = std::fs::remove_file(&path) {
                        self.status = format!("Couldn't remove the folder's settings: {error}");
                    }
                    self.load_settings();
                }
                Task::none()
            }
            Message::Frame(now) => self.on_frame(now),
            Message::TakeShot => window::latest()
                .and_then(window::screenshot)
                .map(Message::Shot),
            Message::Shot(shot) => {
                if let Some(path) = &self.options.shot {
                    save_png(path, &shot);
                    println!("screenshot: {}", path.display());
                }
                if self.options.exit { iced::exit() } else { Task::none() }
            }
        }
    }

    fn on_frame(&mut self, now: Instant) -> Task<Message> {
        let total = self.tab().map_or(0, |t| t.doc.lines.len());
        let Some(auto) = &mut self.auto else {
            return Task::none();
        };
        if let Some(last) = auto.last {
            auto.intervals.push((now - last).as_secs_f32() * 1000.0);
        }
        auto.last = Some(now);
        auto.frames += 1;
        auto.y += self.options.step;

        let end = total as f32 * LINE_HEIGHT - 800.0;
        if auto.frames < AUTOSCROLL_FRAMES && auto.y < end {
            return scroll_code(0.0, auto.y);
        }

        viewer::RECORD.store(false, Ordering::Relaxed);
        let draws = viewer::DRAW_MICROS.lock().unwrap().clone();
        report("widget draw (ms)", &mut draws.iter().map(|d| d / 1000.0).collect::<Vec<_>>());
        report("frame interval (ms)", &mut auto.intervals);
        println!("scrolled {:.0}px over {} frames", auto.y, auto.frames);
        self.auto = None;
        if self.options.exit { iced::exit() } else { Task::none() }
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = vec![event::listen_with(|event, status, _window| match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                Some(Message::ModifiersChanged(modifiers))
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, repeat, .. })
                if status == event::Status::Ignored =>
            {
                Some(Message::KeyPressed(key, modifiers, repeat))
            }
            Event::Keyboard(keyboard::Event::KeyPressed { key: Key::Named(Named::Escape), .. })
                if status == event::Status::Captured =>
            {
                Some(Message::EscapeInTextBox)
            }
            Event::Window(window::Event::Resized(_)) => Some(Message::WindowResized),
            Event::Window(window::Event::CloseRequested) => Some(Message::Exit),
            Event::Window(window::Event::Focused) => Some(Message::WindowFocused),
            Event::Window(window::Event::Unfocused) => {
                Some(Message::ModifiersChanged(Modifiers::default()))
            }
            _ => None,
        })];
        if let Some(root) = &self.root {
            subscriptions.push(Subscription::run_with((root.clone(), true), watch_path));
        }
        let outside: BTreeSet<PathBuf> = self
            .tabs
            .iter()
            .filter(|tab| !self.root.as_ref().is_some_and(|root| tab.path.starts_with(root)))
            .filter_map(|tab| tab.path.parent().map(Path::to_path_buf))
            .collect();
        for dir in outside {
            subscriptions.push(Subscription::run_with((dir, false), watch_path));
        }
        if self.auto.is_some() {
            subscriptions.push(window::frames().map(Message::Frame));
        }
        if matches!(self.suggestion, Some(Suggestion { state: SuggestionState::Loading, .. })) {
            subscriptions.push(window::frames().map(|_| Message::SpinnerFrame));
        }
        if self.auto_update && update::available() {
            let ticks = Subscription::run_with(UPDATE_INTERVAL, |every| update::ticks(*every));
            subscriptions.push(ticks.map(|()| Message::AutoCheckForUpdates));
        }
        Subscription::batch(subscriptions)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let body: Element<'_, Message> = if let Some(plan) = &self.plan {
            self.plan_page(plan)
        } else if self.root.is_none() && self.tabs.is_empty() && !self.show_settings {
                self.welcome()
            } else {
                let mut main = column![self.tab_bar()];
                if self.show_settings {
                    main = main.push(self.settings_page());
                } else {
                    main = main.push(self.code_area());
                }
                main = main.push(self.status_bar());
                let mut layout = row![];
                if self.root.is_some() && self.show_sidebar {
                    layout = layout.push(self.sidebar());
                }
                layout.push(main.width(Fill)).into()
            };

        let page = container(column![self.title_bar(), body])
            .width(Fill)
            .height(Fill)
            .style(self.background_style(0.0));
        let mut layers = stack![page].width(Fill).height(Fill);
        if !self.maximized {
            layers = layers.push(self.resize_edges());
        }
        if let Some(menu) = self.menu {
            layers = layers.push(self.menu_overlay(menu));
        }
        if self.palette_open {
            layers = layers.push(self.command_palette());
        }
        if let Some((version, notes)) = &self.whats_new {
            layers = layers.push(self.whats_new_dialog(version, notes));
        }
        if let Some(concerns) = &self.confirm_exit {
            layers = layers.push(self.exit_dialog(concerns));
        }
        layers.into()
    }

    fn command_palette(&self) -> Element<'_, Message> {
        let matches = self.palette_matches();
        let selected = self.palette_selected.min(matches.len().saturating_sub(1));
        let first = (selected + 1).saturating_sub(PALETTE_ROWS);
        let highlight = mix(self.background, self.palette.foreground, 0.20);
        let flat = self.flat_button(false);
        let dim = Color { a: 0.6, ..self.palette.foreground };
        let rows = matches.into_iter().enumerate().skip(first).take(PALETTE_ROWS).map(|(index, (label, shortcut, _))| {
            let chosen = index == selected;
            button(row![text(label).size(13).width(Fill), text(shortcut).size(11).color(dim)].align_y(iced::Center))
                .width(Fill)
                .padding([4, 10])
                .style(move |theme: &Theme, status| {
                    let mut style = flat(theme, status);
                    if chosen {
                        style.background = Some(Background::Color(highlight));
                    }
                    style
                })
                .on_press(Message::PaletteRun(index))
                .into()
        });
        let list: Element<'_, Message> = column(rows).into();
        let empty = self.palette_matches().is_empty();
        let mut body = column![
            text_input("Type a command", &self.palette_query)
                .id(Id::new(PALETTE_INPUT))
                .on_input(Message::PaletteInput)
                .on_submit(Message::PaletteRun(selected))
                .size(14)
                .padding([5, 8]),
            list,
        ]
        .spacing(4);
        if empty {
            body = body.push(container(text("No matching commands.").size(12)).padding([4, 10]));
        }
        let boxed = container(body).padding(5).width(PALETTE_WIDTH).style(self.panel_style());
        let backdrop = opaque(mouse_area(space::horizontal().width(Fill).height(Fill)).on_press(Message::ClosePalette));
        stack![
            backdrop,
            container(opaque(boxed))
                .width(Fill)
                .align_x(iced::Center)
                .padding(Padding { top: MENU_BAR_HEIGHT + 6.0, ..Padding::ZERO }),
        ]
        .width(Fill)
        .height(Fill)
        .into()
    }

    fn whats_new_dialog<'a>(&'a self, version: &'a str, notes: &'a [NoteLine]) -> Element<'a, Message> {
        let lines = notes.iter().map(|line| -> Element<'a, Message> {
            match line {
                NoteLine::Heading(heading) => text(heading.as_str()).size(14).shaping(text::Shaping::Advanced).into(),
                NoteLine::Bullet(item) => row![text("•").size(13), text(item.as_str()).size(13).shaping(text::Shaping::Advanced)]
                    .spacing(6)
                    .into(),
                NoteLine::Text(line) => text(line.as_str()).size(13).shaping(text::Shaping::Advanced).into(),
            }
        });
        let shade = move |_: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.45))),
            ..Default::default()
        };
        opaque(
            container(
                container(
                    column![
                        text(format!("What's new in v{version}")).size(18),
                        container(scrollable(column(lines).spacing(6).width(Fill))).max_height(360),
                        row![
                            space::horizontal(),
                            button(text("Close").size(12)).padding([5, 14]).on_press(Message::CloseWhatsNew),
                        ],
                    ]
                    .spacing(14),
                )
                .padding(18)
                .width(520)
                .style(self.panel_style()),
            )
            .center(Fill)
            .style(shade),
        )
    }

    fn exit_dialog<'a>(&'a self, concerns: &'a [&'static str]) -> Element<'a, Message> {
        let mut body = column![text("Exit without finishing?").size(16)].spacing(6);
        for concern in concerns {
            body = body.push(text(*concern).size(13));
        }
        let buttons = row![
            space::horizontal(),
            button(text("Keep working").size(12))
                .padding([5, 14])
                .style(button::secondary)
                .on_press(Message::ExitCancelled),
            button(text("Exit anyway").size(12))
                .padding([5, 14])
                .style(button::danger)
                .on_press(Message::ExitConfirmed),
        ]
        .spacing(8);
        let shade = move |_: &Theme| container::Style {
            background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.45))),
            ..Default::default()
        };
        opaque(
            container(
                container(column![body, buttons].spacing(16))
                    .padding(18)
                    .width(380)
                    .style(self.panel_style()),
            )
            .center(Fill)
            .style(shade),
        )
    }

    fn toggle_style(&self, on: bool) -> impl Fn(&Theme, button::Status) -> button::Style + Copy + use<> {
        let flat = self.flat_button(false);
        let tint = mix(self.background, Color::from_rgb8(0x0e, 0x63, 0x9c), 0.75);
        let foreground = self.palette.foreground;
        move |theme: &Theme, status: button::Status| match status {
            button::Status::Disabled => flat(theme, status),
            _ if on => button::Style {
                background: Some(Background::Color(tint)),
                text_color: foreground,
                border: Border { radius: 4.0.into(), ..Border::default() },
                ..Default::default()
            },
            _ => flat(theme, status),
        }
    }

    fn option_button<'a>(&self, label: &'static str, tip: &'static str, on: bool, message: Message) -> Element<'a, Message> {
        let target = button(text(label).size(11).font(Font::with_name("Consolas")))
            .padding([4, 5])
            .style(self.toggle_style(on))
            .on_press(message);
        self.tip(target, tip)
    }

    fn can_save_rules(&self) -> bool {
        self.rules_dirty() && self.edited_rules.errors.is_empty()
    }

    fn find_bar(&self, room: f32) -> Element<'_, Message> {
        let query = &self.find_query;
        let count = self.find_matches.len();
        let label = match (&self.find_error, self.find_current) {
            (Some(_), _) => "Invalid pattern".to_owned(),
            (None, _) if query.text.is_empty() => String::new(),
            (None, None) => "No results".to_owned(),
            (None, Some(current)) => {
                let cap = if count >= find::MAX_FILE_MATCHES { "+" } else { "" };
                format!("{} of {count}{cap}", current + 1)
            }
        };
        let flat = self.flat_button(false);
        let step = |icon: char, tip: &'static str, forward: bool| {
            let target = button(symbol(icon, 10.0))
                .padding([6, 7])
                .style(flat)
                .on_press_maybe((count > 0).then_some(Message::FindStep(forward)));
            self.tip(target, tip)
        };
        let close = button(symbol(glyph::CLOSE, 10.0)).padding([6, 7]).style(flat).on_press(Message::CloseFind);
        let bar = row![
            text_input("Find", &query.text)
                .id(Id::new(FIND_INPUT))
                .on_input(Message::FindInput)
                .on_submit(Message::FindSubmit)
                .size(13)
                .padding([3, 6])
                .width(Fill),
            self.option_button("Aa", "Match case", query.match_case, Message::FindToggle(FindOption::Case)),
            self.option_button("ab", "Whole word", query.whole_word, Message::FindToggle(FindOption::Word)),
            self.option_button(".*", "Regular expression", query.regex, Message::FindToggle(FindOption::Regex)),
            container(text(label).size(11).wrapping(text::Wrapping::None)).width(78).align_x(iced::Center),
            step(glyph::CHEVRON_UP, "Previous (Shift+F3)", false),
            step(glyph::CHEVRON_DOWN, "Next (F3)", true),
            self.tip(close, "Close (Esc)"),
        ]
        .spacing(2)
        .align_y(iced::Center);
        let boxed = container(bar)
            .padding([3, 4])
            .width((room - SCROLLBAR_ROOM * 2.0).min(FIND_BAR_WIDTH))
            .style(self.panel_style());
        container(opaque(boxed))
            .width(Fill)
            .align_x(iced::Right)
            .padding(Padding { top: TOOLBAR_GAP, right: SCROLLBAR_ROOM + 4.0, ..Padding::ZERO })
            .into()
    }

    fn search_panel(&self) -> Element<'_, Message> {
        let query = &self.search_query;
        let files = self.search_files.len();
        let summary = if let Some(error) = &self.search_error {
            error.clone()
        } else if query.text.is_empty() {
            String::new()
        } else if self.search_total == 0 {
            if self.search_running { "Searching…".to_owned() } else { "No results".to_owned() }
        } else {
            let plural = |n: usize, word: &str| format!("{n} {word}{}", if n == 1 { "" } else { "s" });
            let found = format!("{} in {}", plural(self.search_total, "result"), plural(files, "file"));
            if self.search_total >= find::MAX_FOLDER_MATCHES {
                format!("First {found}")
            } else if self.search_running {
                format!("{found}…")
            } else {
                found
            }
        };
        let controls = column![
            text_input("Find in all files", &query.text)
                .id(Id::new(SEARCH_INPUT))
                .on_input(Message::SearchInput)
                .size(13)
                .padding([3, 6]),
            row![
                text(summary).size(11).width(Fill),
                self.option_button("Aa", "Match case", query.match_case, Message::SearchToggle(FindOption::Case)),
                self.option_button("ab", "Whole word", query.whole_word, Message::SearchToggle(FindOption::Word)),
                self.option_button(".*", "Regular expression", query.regex, Message::SearchToggle(FindOption::Regex)),
            ]
            .spacing(2)
            .align_y(iced::Center),
        ]
        .spacing(4)
        .padding(Padding { top: 0.0, right: 10.0, bottom: 6.0, left: 10.0 });

        let total = self.search_rows.len();
        let (offset, height) = self.search_scroll;
        let first = ((offset / SEARCH_ROW_HEIGHT).floor().max(0.0) as usize).min(total);
        let end = (first + (height / SEARCH_ROW_HEIGHT).ceil() as usize + 2).min(total);
        let rows = self.search_rows[first..end].iter().map(|row| self.search_row(*row));
        let list = column![
            space::vertical().height(first as f32 * SEARCH_ROW_HEIGHT),
            column(rows).width(Fill),
            space::vertical().height((total - end) as f32 * SEARCH_ROW_HEIGHT),
        ]
        .width(Fill);
        let results = scrollable(list).id(Id::new(SEARCH_SCROLL)).on_scroll(Message::SearchScrolled).width(Fill).height(Fill);
        column![controls, results].into()
    }

    fn search_row(&self, row_kind: SearchRow) -> Element<'_, Message> {
        let foreground = self.palette.foreground;
        let dim = Color { a: 0.6, ..foreground };
        match row_kind {
            SearchRow::File(index) => {
                let file = &self.search_files[index];
                let name = file.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let folder = Path::new(&file.label)
                    .parent()
                    .map(|parent| parent.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let chevron = if self.search_collapsed.contains(&file.path) {
                    glyph::CHEVRON_RIGHT
                } else {
                    glyph::CHEVRON_DOWN
                };
                let entry = Entry { path: file.path.clone(), name: name.clone(), is_dir: false, depth: 0, ignored: false };
                let [r, g, b, _] = self.background.into_rgba8();
                let light = 0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 128.0;
                let icon = self.icon_for(&entry, light);
                let line = row![
                    container(symbol(chevron, 8.0)).width(12).align_x(iced::Center),
                    self.icon_view(&icon, false),
                    text(name).size(13).shaping(text::Shaping::Advanced).wrapping(text::Wrapping::None),
                    container(text(folder).size(11).color(dim).shaping(text::Shaping::Advanced).wrapping(text::Wrapping::None))
                        .width(Fill)
                        .clip(true),
                    text(file.hits.len().to_string()).size(11).color(dim),
                ]
                .spacing(5)
                .height(Fill)
                .align_y(iced::Center);
                button(line)
                    .width(Fill)
                    .height(SEARCH_ROW_HEIGHT)
                    .padding([0, 6])
                    .style(self.flat_button(false))
                    .on_press(Message::SearchToggleFile(file.path.clone()))
                    .into()
            }
            SearchRow::Hit(file, hit) => {
                let found = &self.search_files[file].hits[hit];
                let highlight = Color::from_rgba(0.2, 0.65, 1.0, 0.42);
                let spans: Vec<Span<'_, (), Font>> = vec![
                    Span::new(found.before.as_str()).color(dim),
                    Span::new(found.matched.as_str()).background(highlight),
                    Span::new(found.after.as_str()).color(dim),
                ];
                let line = row![
                    text((found.line + 1).to_string()).size(11).color(dim).width(36).align_x(iced::Right),
                    container(rich_text(spans).size(12).wrapping(text::Wrapping::None)).width(Fill).clip(true),
                ]
                .spacing(8)
                .height(Fill)
                .align_y(iced::Center);
                button(line)
                    .width(Fill)
                    .height(SEARCH_ROW_HEIGHT)
                    .padding([0, 6])
                    .style(self.flat_button(false))
                    .on_press(Message::SearchOpenHit(file, hit))
                    .into()
            }
        }
    }

    fn sidebar_footer(&self) -> Element<'_, Message> {
        let dim = Color { a: 0.6, ..self.palette.foreground };
        let action = |label: String, message: Message| -> Element<'_, Message> {
            button(text(label).size(11).wrapping(text::Wrapping::None))
                .padding([1, 6])
                .on_press(message)
                .into()
        };
        let update: Element<'_, Message> = match &self.update {
            UpdateState::Available(release) => {
                action(format!("Update to v{}", release.version), Message::UpdateDownload(release.clone()))
            }
            UpdateState::Downloading { percent, .. } => {
                text(format!("Downloading {percent}%")).size(11).color(dim).into()
            }
            UpdateState::Ready { .. } => action("Restart to update".to_owned(), Message::UpdateRestart),
            _ => space::horizontal().width(0).into(),
        };
        container(
            row![
                text(format!("v{}  ·  {}", update::VERSION, update::COMMIT))
                    .size(12)
                    .color(dim)
                    .width(Fill)
                    .wrapping(text::Wrapping::None),
                update,
            ]
            .height(SIDEBAR_FOOTER_ROW)
            .align_y(iced::Center),
        )
        .padding([3, 10])
        .width(Fill)
        .into()
    }

    fn updates_section(&self) -> Element<'_, Message> {
        let title = column![
            text("Updates").size(15),
            text(format!("Version {} ({})", update::VERSION, update::COMMIT)).size(12),
        ]
        .spacing(2);
        if !update::available() {
            return column![title, text("This copy wasn't built by the release workflow, so it can't update itself.").size(12)]
                .spacing(8)
                .into();
        }
        let state = match &self.update {
            UpdateState::Idle => String::new(),
            UpdateState::Checking => "Checking…".to_owned(),
            UpdateState::UpToDate => "You're on the latest version.".to_owned(),
            UpdateState::Available(release) => format!("Version {} is available.", release.version),
            UpdateState::Downloading { version, percent, .. } => format!("Downloading {version}: {percent}%"),
            UpdateState::Ready { version, .. } => format!("Version {version} installs when you close the app."),
            UpdateState::Failed(error) => error.clone(),
        };
        let can_check = !matches!(
            self.update,
            UpdateState::Checking | UpdateState::Downloading { .. } | UpdateState::Ready { .. }
        );
        column![
            title,
            checkbox(self.auto_update)
                .label("Find and download updates automatically")
                .text_size(13)
                .size(15)
                .on_toggle(Message::SetAutoUpdate),
            row![
                button(text("Check for updates").size(12))
                    .padding([3, 12])
                    .on_press_maybe(can_check.then_some(Message::CheckForUpdates)),
                text(state).size(12),
            ]
            .spacing(10)
            .align_y(iced::Center),
        ]
        .spacing(8)
        .into()
    }

    fn branch_row(&self) -> Element<'_, Message> {
        let Some(repo) = &self.repo else {
            return space::vertical().height(0).into();
        };
        let tint = Color { a: 0.75, ..self.palette.foreground };
        let branch = svg(svg::Handle::from_memory(BRANCH))
            .width(12)
            .height(12)
            .style(move |_: &Theme, _| svg::Style { color: Some(tint) });
        let changes = repo.files.len();
        let name = text(repo.branch.as_str())
            .size(11)
            .color(tint)
            .shaping(text::Shaping::Advanced)
            .wrapping(text::Wrapping::None);
        let toggle = button(text("Changed only").size(11))
            .padding([2, 6])
            .style(self.toggle_style(self.only_changed))
            .on_press(Message::ToggleOnlyChanged);
        row![
            branch,
            container(name).width(Fill).clip(true),
            text(changes.to_string()).size(11).color(tint),
            self.tip(toggle, "Clean only comments on lines changed since the last commit"),
        ]
        .spacing(5)
        .align_y(iced::Center)
        .padding(Padding { top: 0.0, right: 10.0, bottom: 5.0, left: 10.0 })
        .into()
    }

    fn background_style(&self, lift: f32) -> impl Fn(&Theme) -> container::Style + use<> {
        let background = mix(self.background, self.palette.foreground, lift);
        let foreground = self.palette.foreground;
        move |_: &Theme| container::Style {
            background: Some(Background::Color(background)),
            text_color: Some(foreground),
            ..Default::default()
        }
    }

    fn panel_style(&self) -> impl Fn(&Theme) -> container::Style + use<> {
        let background = mix(self.background, self.palette.foreground, 0.10);
        let foreground = self.palette.foreground;
        let edge = mix(self.background, self.palette.foreground, 0.28);
        move |_: &Theme| container::Style {
            background: Some(Background::Color(background)),
            text_color: Some(foreground),
            border: Border { color: edge, width: 1.0, radius: 6.0.into() },
            shadow: Shadow {
                color: Color::from_rgba(0.0, 0.0, 0.0, 0.45),
                offset: Vector::new(0.0, 3.0),
                blur_radius: 10.0,
            },
            ..Default::default()
        }
    }

    fn flat_button(&self, danger: bool) -> impl Fn(&Theme, button::Status) -> button::Style + Copy + use<> {
        let foreground = self.palette.foreground;
        let hover = mix(self.background, foreground, 0.20);
        let press = mix(self.background, foreground, 0.28);
        let red = Color::from_rgb8(0xc7, 0x4e, 0x39);
        move |_: &Theme, status: button::Status| {
            let (background, text_color) = match status {
                button::Status::Hovered if danger => (Some(red), Color::WHITE),
                button::Status::Pressed if danger => (Some(red), Color::WHITE),
                button::Status::Hovered => (Some(hover), foreground),
                button::Status::Pressed => (Some(press), foreground),
                button::Status::Disabled => (None, Color { a: 0.35, ..foreground }),
                button::Status::Active => (None, foreground),
            };
            button::Style {
                background: background.map(Background::Color),
                text_color,
                border: Border { radius: 4.0.into(), ..Border::default() },
                ..Default::default()
            }
        }
    }

    fn tip<'a>(
        &self,
        content: impl Into<Element<'a, Message>>,
        label: &'a str,
    ) -> Element<'a, Message> {
        tooltip(
            content,
            container(text(label).size(11)).padding([3, 7]).style(self.panel_style()),
            tooltip::Position::Bottom,
        )
        .gap(4)
        .into()
    }

    fn labelled<'a>(&self, icon: char, shortcut: &'static str, label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
        self.labelled_with(symbol(icon, 12.0).into(), shortcut, label)
    }

    fn spinner<'a>(&self) -> Element<'a, Message> {
        let tint = self.palette.foreground;
        let turn = self.spinner_start.elapsed().as_secs_f32().rem_euclid(1.0) * std::f32::consts::TAU;
        svg(svg::Handle::from_memory(SPINNER))
            .width(14)
            .height(14)
            .rotation(Rotation::Floating(Radians(turn)))
            .style(move |_: &Theme, _| svg::Style { color: Some(tint) })
            .into()
    }

    fn sparkle<'a>(&self, enabled: bool) -> Element<'a, Message> {
        let tint = Color { a: if enabled { 1.0 } else { 0.35 }, ..self.palette.foreground };
        svg(svg::Handle::from_memory(SPARKLE))
            .width(14)
            .height(14)
            .style(move |_: &Theme, _| svg::Style { color: Some(tint) })
            .into()
    }

    fn labelled_with<'a>(&self, icon: Element<'a, Message>, shortcut: &'static str, label: impl text::IntoFragment<'a>) -> Element<'a, Message> {
        let slot: Element<'a, Message> = if self.ctrl_held && !shortcut.is_empty() {
            text(shortcut)
                .size(10)
                .font(Font { weight: Weight::Bold, ..Font::with_name("Segoe UI") })
                .into()
        } else {
            icon
        };
        row![container(slot).width(20).align_x(iced::Center), text(label).size(12)]
            .spacing(3)
            .align_y(iced::Center)
            .into()
    }

    fn resize_edges(&self) -> Element<'_, Message> {
        use mouse::Interaction::{
            ResizingDiagonallyDown as Falling, ResizingDiagonallyUp as Rising,
            ResizingHorizontally as Sideways, ResizingVertically as Upright,
        };
        let grip = |edge: Edge, cursor, width: Length, height: Length| -> Element<'_, Message> {
            mouse_area(space::horizontal().width(width).height(height))
                .interaction(cursor)
                .on_press(Message::WindowResize(edge))
                .into()
        };
        let thin = Length::Fixed(RESIZE_EDGE);
        let corner = Length::Fixed(RESIZE_CORNER);
        column![
            row![
                grip(Edge::NorthWest, Falling, corner, thin),
                grip(Edge::North, Upright, Fill, thin),
                grip(Edge::NorthEast, Rising, corner, thin),
            ],
            row![
                grip(Edge::West, Sideways, thin, Fill),
                space::horizontal().width(Fill).height(Fill),
                grip(Edge::East, Sideways, thin, Fill),
            ]
            .height(Fill),
            row![
                grip(Edge::SouthWest, Rising, corner, thin),
                grip(Edge::South, Upright, Fill, thin),
                grip(Edge::SouthEast, Falling, corner, thin),
            ],
        ]
        .into()
    }

    fn window_button(&self, icon: char, danger: bool, message: Message) -> Element<'_, Message> {
        let flat = self.flat_button(danger);
        button(container(symbol(icon, 10.0)).center(Fill))
            .width(WINDOW_BUTTON_WIDTH)
            .height(Fill)
            .padding(0)
            .style(move |theme: &Theme, status| button::Style {
                border: Border::default(),
                ..flat(theme, status)
            })
            .on_press(message)
            .into()
    }

    fn title_bar(&self) -> Element<'_, Message> {
        let titles = MENUS.iter().map(|&(menu, title, width)| {
            let flat = self.flat_button(false);
            let open = self.menu == Some(menu);
            let highlight = mix(self.background, self.palette.foreground, 0.20);
            let entry = button(text(title).size(12).width(Fill).center())
                .width(width)
                .padding([4, 0])
                .style(move |theme: &Theme, status| {
                    let mut style = flat(theme, status);
                    if open {
                        style.background = Some(Background::Color(highlight));
                    }
                    style
                })
                .on_press_maybe(self.plan.is_none().then_some(Message::OpenMenu(menu)));
            mouse_area(entry).on_enter(Message::HoverMenu(menu)).into()
        });
        let mut drag_area = mouse_area(space::horizontal().width(Fill).height(Fill))
            .on_press(Message::TitleBarPressed)
            .on_release(Message::TitleBarReleased);
        if self.title_armed {
            drag_area = drag_area
                .on_move(|_| Message::TitleBarDragged)
                .on_exit(Message::TitleBarDragged);
        }
        let title = container(
            text(self.title())
                .size(12)
                .color(Color { a: 0.8, ..self.palette.foreground })
                .shaping(text::Shaping::Advanced)
                .wrapping(text::Wrapping::None),
        )
        .center(Fill)
        .padding([0.0, TITLE_SIDE_ROOM])
        .clip(true);
        let maximize = if self.maximized { glyph::WINDOW_RESTORE } else { glyph::WINDOW_MAXIMIZE };
        let app_icon = container(svg(svg::Handle::from_memory(APP_ICON)).width(16).height(16))
            .width(APP_ICON_SLOT)
            .height(Fill)
            .align_x(iced::Center)
            .align_y(iced::Center);
        let controls = row![
            app_icon,
            container(row(titles).align_y(iced::Center).padding([0, 4])).center_y(Fill),
            space::horizontal(),
            self.window_button(glyph::WINDOW_MINIMIZE, false, Message::WindowMinimize),
            self.window_button(maximize, false, Message::WindowToggleMaximize),
            self.window_button(glyph::WINDOW_CLOSE, true, Message::Exit),
        ];
        container(stack![drag_area, title, controls].width(Fill).height(Fill))
            .width(Fill)
            .height(MENU_BAR_HEIGHT)
            .style(self.background_style(0.06))
            .into()
    }

    fn menu_overlay(&self, menu: Menu) -> Element<'_, Message> {
        let left = APP_ICON_SLOT
            + 4.0
            + MENUS
                .iter()
                .take_while(|(candidate, ..)| *candidate != menu)
                .map(|(.., width)| width)
                .sum::<f32>();
        let dropdown = container(opaque(self.menu_items(menu)))
            .padding(4)
            .width(MENU_WIDTH)
            .style(self.panel_style());
        let backdrop = opaque(
            mouse_area(space::horizontal().width(Fill).height(Fill)).on_press(Message::CloseMenu),
        );
        column![
            space::vertical().height(MENU_BAR_HEIGHT),
            stack![
                backdrop,
                container(dropdown).padding(Padding { left, ..Padding::ZERO }),
            ]
            .width(Fill)
            .height(Fill),
        ]
        .into()
    }

    fn menu_items(&self, menu: Menu) -> Element<'_, Message> {
        let flat = self.flat_button(false);
        let dim = Color { a: 0.6, ..self.palette.foreground };
        let item = |label: &'static str, shortcut: &'static str, message: Option<Message>| -> Element<'_, Message> {
            let enabled = message.is_some();
            let mut hint = text(shortcut).size(11);
            if enabled {
                hint = hint.color(dim);
            }
            button(row![text(label).size(12).width(Fill), hint].align_y(iced::Center))
                .width(Fill)
                .padding([4, 10])
                .style(flat)
                .on_press_maybe(message.map(|m| Message::MenuItem(Box::new(m))))
                .into()
        };
        let edge = mix(self.background, self.palette.foreground, 0.22);
        let separator = || -> Element<'_, Message> {
            container(
                container(space::horizontal())
                    .width(Fill)
                    .height(1)
                    .style(move |_: &Theme| container::Style {
                        background: Some(Background::Color(edge)),
                        ..Default::default()
                    }),
            )
            .padding([4, 6])
            .into()
        };

        column(self.menu_entries(menu).into_iter().map(|entry| match entry {
            MenuEntry::Item(label, shortcut, message) => item(label, shortcut, message),
            MenuEntry::Separator => separator(),
        }))
        .into()
    }

    fn menu_entries(&self, menu: Menu) -> Vec<MenuEntry> {
        let a = self.available();
        let has_tab = self.active.is_some();
        let has_hunks = self.tab().is_some_and(|t| !t.doc.hunks.is_empty());
        let selected = self.tab().is_some_and(|t| t.selected.is_some());
        let when = |enabled: bool, message: Message| enabled.then_some(message);

        let mut items = Vec::new();
        match menu {
            Menu::File => {
                items.push(MenuEntry::Item("Open File…", "Ctrl+O", Some(Message::PickFile)));
                items.push(MenuEntry::Item("Open Folder…", "Ctrl+Shift+O", Some(Message::PickFolder)));
                items.push(MenuEntry::Separator);
                items.push(MenuEntry::Item("Close File", "Ctrl+W", when(has_tab, Message::CloseFile)));
                if self.root.is_some() {
                    items.push(MenuEntry::Item("Close Folder", "", Some(Message::CloseFolder)));
                }
                items.push(MenuEntry::Separator);
                items.push(MenuEntry::Item("Exit", "", Some(Message::Exit)));
            }
            Menu::Edit => {
                items.push(MenuEntry::Item("Undo", "Ctrl+Z", when(self.can_undo(), Message::Undo)));
                items.push(MenuEntry::Separator);
                items.push(MenuEntry::Item("Fix", "F", when(a.fix, Message::Do(Action::Fix))));
                items.push(MenuEntry::Item("Ignore", "I", when(a.ignore, Message::Do(Action::Ignore))));
                items.push(MenuEntry::Item("Squash", "S", when(a.squash, Message::Do(Action::Squash))));
                items.push(MenuEntry::Item("Delete", "Del", when(a.delete, Message::Do(Action::Delete))));
                items.push(MenuEntry::Item("Reword", self.reword_shortcut(), when(a.reword, Message::Do(Action::Reword))));
                items.push(MenuEntry::Item("Suggest", "A", when(a.suggest, Message::Do(Action::Suggest))));
                items.push(MenuEntry::Separator);
                items.push(MenuEntry::Item(self.fix_mode.menu_label(), "", when(self.can_fix_all(), Message::FixAll)));
                items.push(MenuEntry::Item("Clean Folder…", "", when(self.can_clean_folder(), Message::CleanFolder)));
                let can_undo_folder = !self.busy && !self.folder_undo.is_empty();
                items.push(MenuEntry::Item("Undo Folder Cleanup", "", when(can_undo_folder, Message::UndoFolder)));
                items.push(MenuEntry::Separator);
                items.push(MenuEntry::Item("Find", "Ctrl+F", when(has_tab, Message::OpenFind)));
                items.push(MenuEntry::Item("Find in All Files", "Ctrl+Shift+F", when(self.root.is_some(), Message::OpenSearch)));
            }
            Menu::View => {
                items.push(MenuEntry::Item("Go Through All", "G", when(has_hunks, Message::GoThroughAll)));
                items.push(MenuEntry::Item("Next Comment", "N", when(has_hunks, Message::Next)));
                items.push(MenuEntry::Item("Previous Comment", "P", when(selected, Message::Previous)));
                items.push(MenuEntry::Separator);
                let sidebar = if self.show_sidebar { "Hide Sidebar" } else { "Show Sidebar" };
                items.push(MenuEntry::Item(sidebar, "Ctrl+B", when(self.root.is_some(), Message::ToggleSidebar)));
                items.push(MenuEntry::Item("Settings", "Ctrl+,", Some(Message::ToggleSettings)));
                items.push(MenuEntry::Item("Command Palette", "Ctrl+Shift+P", Some(Message::OpenPalette)));
                items.push(MenuEntry::Separator);
                let only_changed = if self.only_changed { "✓ Only Changed Lines" } else { "Only Changed Lines" };
                items.push(MenuEntry::Item(only_changed, "", when(self.repo.is_some(), Message::ToggleOnlyChanged)));
                if update::available() {
                    items.push(MenuEntry::Item("Check for Updates", "", Some(Message::CheckForUpdates)));
                }
            }
        }
        items
    }

    fn welcome(&self) -> Element<'_, Message> {
        center(
            column![
                text("CommentCleanup").size(24),
                row![
                    button(self.labelled(glyph::FOLDER_OPEN, "", "Open folder…"))
                        .padding([5, 12])
                        .on_press(Message::PickFolder),
                    button(self.labelled(glyph::FILE, "", "Open file…"))
                        .padding([5, 12])
                        .style(button::secondary)
                        .on_press(Message::PickFile),
                ]
                .spacing(8),
                text(&self.status).size(12),
            ]
            .spacing(14)
            .align_x(iced::Center),
        )
        .into()
    }

    fn icon_view(&self, icon: &Icon, dim: bool) -> Element<'_, Message> {
        let fade = if dim { 0.5 } else { 1.0 };
        let cell = |content: Element<'static, Message>| -> Element<'static, Message> {
            container(content)
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .align_x(iced::Center)
                .align_y(iced::Center)
                .into()
        };
        match icon {
            Icon::Svg(handle) => svg(handle.clone())
                .width(ICON_SIZE)
                .height(ICON_SIZE)
                .opacity(fade)
                .into(),
            Icon::Glyph { glyph, color, font, size } => {
                let color = color.unwrap_or(Color { a: 0.75, ..self.palette.foreground });
                cell(
                    text(glyph.to_string())
                        .font(*font)
                        .size(*size)
                        .line_height(text::LineHeight::Absolute(ICON_SIZE.into()))
                        .shaping(text::Shaping::Advanced)
                        .wrapping(text::Wrapping::None)
                        .color(Color { a: color.a * fade, ..color })
                        .into(),
                )
            }
            Icon::Badge { label, color } => {
                let background = Color { a: fade, ..*color };
                let luminance = 0.299 * color.r + 0.587 * color.g + 0.114 * color.b;
                let ink = if luminance > 0.55 { Color::BLACK } else { Color::WHITE };
                container(
                    text(*label)
                        .size(8.5)
                        .font(Font { weight: Weight::Bold, ..Font::with_name("Segoe UI") })
                        .line_height(text::LineHeight::Absolute(13.0.into()))
                        .wrapping(text::Wrapping::None)
                        .color(Color { a: fade, ..ink }),
                )
                .width(ICON_SIZE)
                .height(13)
                .align_x(iced::Center)
                .align_y(iced::Center)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(background)),
                    border: Border { radius: 3.0.into(), ..Border::default() },
                    ..Default::default()
                })
                .into()
            }
        }
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let folder = self
            .root
            .as_ref()
            .and_then(|r| r.file_name())
            .map(|n| n.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let open = button(symbol(glyph::FOLDER_OPEN, 14.0))
            .padding([3, 6])
            .style(self.flat_button(false))
            .on_press(Message::PickFolder);
        let clean = button(symbol(glyph::FIX_ALL, 14.0))
            .padding([3, 6])
            .style(self.flat_button(false))
            .on_press_maybe(self.can_clean_folder().then_some(Message::CleanFolder));
        let search = button(symbol(glyph::SEARCH, 13.0))
            .padding([3, 6])
            .style(self.toggle_style(self.side_panel == SidePanel::Search))
            .on_press(Message::ToggleSearchPanel);
        let header = row![
            text(folder).size(12).width(Fill),
            self.tip(search, "Find in all files (Ctrl+Shift+F)"),
            self.tip(clean, "Clean folder…"),
            self.tip(open, "Open folder…"),
        ]
            .align_y(iced::Center)
            .padding([6, 10]);

        let active_path = self.tab().map(|t| t.path.as_path());
        let highlight = mix(self.background, self.palette.foreground, 0.16);
        let foreground = self.palette.foreground;
        let hover = mix(self.background, self.palette.foreground, 0.10);

        let items = self.entries.iter().enumerate().map(|(index, entry)| {
            let git = self.git_state(&entry.path);
            let holds_changes = entry.is_dir && self.repo_dirs.contains(&entry.path);
            let label_color = if entry.ignored {
                Color { a: 0.45, ..foreground }
            } else if let Some(state) = git {
                git_color(state)
            } else if holds_changes {
                git_color(FileState::Modified)
            } else {
                foreground
            };
            let chevron: Element<'_, Message> = if entry.is_dir {
                let glyph = if self.expanded.contains(&entry.path) {
                    glyph::CHEVRON_DOWN
                } else {
                    glyph::CHEVRON_RIGHT
                };
                symbol(glyph, 8.0).color(label_color).into()
            } else {
                space::horizontal().into()
            };
            let icon = self.entry_icons.get(index).map_or_else(
                || space::horizontal().width(ICON_SIZE).into(),
                |icon| self.icon_view(icon, entry.ignored),
            );
            let is_active = active_path == Some(entry.path.as_path());
            let label = text(entry.name.as_str())
                .size(13)
                .color(label_color)
                .shaping(text::Shaping::Advanced)
                .wrapping(text::Wrapping::None);
            let mut line = row![container(chevron).width(12).align_x(iced::Center), icon, label]
                .spacing(5)
                .align_y(iced::Center);
            let count = self.counts.get(&entry.path);
            if count.is_some() || git.is_some() || holds_changes {
                line = line.push(space::horizontal());
            }
            if let Some(count) = count {
                line = line.push(text(count.to_string()).size(11).color(Color { a: 0.6, ..foreground }));
            }
            if let Some(state) = git {
                line = line.push(text(state.letter()).size(11).color(git_color(state)));
            } else if holds_changes {
                line = line.push(text("●").size(8).color(git_color(FileState::Modified)));
            }
            button(line.padding(Padding { right: 6.0, ..Padding::ZERO }))
            .width(Fill)
            .padding(Padding {
                top: 2.0,
                bottom: 2.0,
                left: 6.0 + f32::from(entry.depth) * 14.0,
                right: 4.0,
            })
            .style(move |_: &Theme, status| button::Style {
                background: match status {
                    _ if is_active => Some(Background::Color(highlight)),
                    button::Status::Hovered => Some(Background::Color(hover)),
                    _ => None,
                },
                text_color: foreground,
                ..Default::default()
            })
            .on_press(Message::EntryPressed(index))
            .into()
        });
        let body: Element<'_, Message> = if self.side_panel == SidePanel::Search {
            self.search_panel()
        } else {
            let list: Element<'_, Message> = column(items).width(Fill).into();
            scrollable(list).height(Fill).width(Fill).into()
        };

        container(column![header, self.branch_row(), body, self.sidebar_footer()])
            .width(SIDEBAR_WIDTH)
            .height(Fill)
            .style(self.background_style(0.04))
            .into()
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let foreground = self.palette.foreground;
        let active_bg = self.background;
        let idle_bg = mix(self.background, foreground, 0.06);

        let flat = move |active: bool| {
            move |_: &Theme, status: button::Status| button::Style {
                background: Some(Background::Color(match status {
                    button::Status::Hovered if !active => mix(idle_bg, foreground, 0.06),
                    _ if active => active_bg,
                    _ => idle_bg,
                })),
                text_color: if active {
                    foreground
                } else {
                    Color { a: 0.7, ..foreground }
                },
                border: Border::default(),
                ..Default::default()
            }
        };

        let tabs = self.tabs.iter().enumerate().map(|(index, tab)| {
            let active = self.active == Some(index);
            let label = text(tab.name.as_str())
                .size(13)
                .wrapping(text::Wrapping::None)
                .color_maybe(self.git_state(&tab.path).map(git_color));
            row![
                button(label)
                    .padding(Padding { top: 7.0, bottom: 7.0, left: 12.0, right: 4.0 })
                    .style(flat(active))
                    .on_press(Message::ActivateTab(index)),
                button(symbol(glyph::CLOSE, 9.0))
                    .height(Fill)
                    .padding(Padding { top: 11.0, bottom: 7.0, left: 6.0, right: 8.0 })
                    .style(flat(active))
                    .on_press(Message::CloseTab(index)),
            ]
            .height(32)
            .into()
        });
        let hidden = Scrollbar::new().width(0).scroller_width(0);
        let strip = scrollable(row(tabs).spacing(1))
            .direction(Direction::Horizontal(hidden))
            .width(Fill);

        let reviewing = self.tab().is_some_and(|t| t.review.is_some());
        let has_hunks = self.tab().is_some_and(|t| !t.doc.hunks.is_empty());
        let review_button = if reviewing {
            button(self.labelled(glyph::STOP, if self.suggestion.is_some() { "" } else { "Esc" }, "Stop"))
                .padding([4, 10])
                .style(button::secondary)
                .on_press(Message::StopReview)
        } else {
            button(self.labelled(glyph::PLAY, "G", "Go through all"))
                .padding([4, 10])
                .on_press_maybe(has_hunks.then_some(Message::GoThroughAll))
        };
        let fix_mode = pick_list(&FixMode::ALL[..], Some(self.fix_mode), Message::FixModePicked)
            .text_size(12)
            .padding([4, 8]);
        let fix_all = button(self.labelled(glyph::FIX_ALL, "", self.fix_mode.button(self.fix_count())))
            .padding([4, 10])
            .on_press_maybe(self.can_fix_all().then_some(Message::FixAll));
        let settings = button(symbol(glyph::SETTINGS, 16.0))
            .padding([4, 7])
            .style(self.flat_button(false))
            .on_press(Message::ToggleSettings);

        container(
            row![strip, fix_mode, fix_all, review_button, self.tip(settings, "Settings")]
                .spacing(8)
                .align_y(iced::Center)
                .padding(Padding { right: 8.0, ..Padding::ZERO }),
        )
        .width(Fill)
        .style(self.background_style(0.06))
        .into()
    }

    fn code_area(&self) -> Element<'_, Message> {
        let Some(tab) = self.tab() else {
            return center(text("Choose a file from the sidebar.").size(14)).into();
        };
        let selected = tab.selected.map(|i| i as u32);
        let code = CodeView::new(
            &tab.doc,
            tab.generation,
            self.palette,
            selected,
            &tab.dimmed,
            Message::HunkClicked,
            Message::CodeClicked,
        );
        let code = code.find_results(&self.find_matches, self.find_current);
        let view = scrollable(code)
            .id(Id::new(CODE_SCROLL))
            .direction(Direction::Both {
                vertical: Scrollbar::default(),
                horizontal: Scrollbar::default(),
            })
            .on_scroll(Message::Scrolled)
            .width(Fill)
            .height(Fill);
        let find: Element<'_, Message> = if self.find_open {
            responsive(move |size| self.find_bar(size.width)).into()
        } else {
            space::horizontal().into()
        };
        let toolbar: Element<'_, Message> = if selected.is_some() {
            responsive(move |size| self.floating_toolbar(size)).into()
        } else {
            space::horizontal().into()
        };
        stack![view, find, toolbar].width(Fill).height(Fill).into()
    }

    fn floating_toolbar(&self, size: Size) -> Element<'_, Message> {
        let Some((tab, index, hunk)) = self.selection() else {
            return space::horizontal().into();
        };
        let height = TOOLBAR_HEIGHT
            + if self.reword.is_some() { REWORD_HEIGHT } else { 0.0 }
            + self.suggestion_height();
        let limit = size.height - SCROLLBAR_ROOM;
        let top = hunk.first as f32 * LINE_HEIGHT - tab.offset.y;
        let bottom = (hunk.last + 1) as f32 * LINE_HEIGHT - tab.offset.y;
        let below = bottom + TOOLBAR_GAP;
        let above = top - TOOLBAR_GAP - height;
        let y = if below + height <= limit {
            below
        } else if above >= 0.0 {
            above
        } else {
            limit - height
        };
        let floor = if self.find_open { FIND_BAR_HEIGHT + TOOLBAR_GAP } else { TOOLBAR_GAP };
        let y = y.clamp(floor, (limit - height).max(floor));

        let toolbar = self.toolbar(tab, index, hunk, size.width);
        container(opaque(toolbar))
            .width(Fill)
            .height(Fill)
            .align_x(iced::Right)
            .padding(Padding { top: y, right: SCROLLBAR_ROOM + 4.0, ..Padding::ZERO })
            .into()
    }

    fn toolbar<'a>(&'a self, tab: &'a Tab, index: usize, hunk: &'a Hunk, room: f32) -> Element<'a, Message> {
        let lines = if hunk.first == hunk.last {
            format!("Line {}", hunk.first + 1)
        } else {
            format!("Lines {}–{}", hunk.first + 1, hunk.last + 1)
        };
        let kind = match hunk.kind {
            HunkKind::LineRun if !hunk.whole_line => "Trailing comment",
            HunkKind::LineRun => "Line comment",
            HunkKind::Block => "Block comment",
            HunkKind::Unknown => "Comment",
        };
        let decision = tab.decisions.get(index).unwrap_or(&Decision::Keep);
        let (fix_label, verdict) = match decision {
            Decision::Keep => ("Fix", "No rule applies".to_owned()),
            Decision::Protected(why) => ("Fix", format!("Protected: {why}")),
            Decision::Squash(_) => ("Fix: squash", "Rules say squash".to_owned()),
            Decision::Delete(why) => ("Fix: delete", format!("Rules say delete: {why}")),
        };

        let a = self.available();
        let flat = self.flat_button(false);
        let action = |icon: char, key: &'static str, label: &'static str, action: Action, enabled: bool, danger: bool| {
            let style = self.flat_button(danger);
            button(self.labelled(icon, key, label))
                .padding(Padding { top: 4.0, bottom: 4.0, left: 3.0, right: 8.0 })
                .style(style)
                .on_press_maybe(enabled.then_some(Message::Do(action)))
        };
        let nav = |icon: char, label: &'static str, message: Message| {
            let target = button(symbol(icon, 10.0))
                .padding([6, 7])
                .style(flat)
                .on_press_maybe((!self.busy).then_some(message));
            self.tip(target, label)
        };

        let wide = room >= WIDE_TOOLBAR;
        let mut bar = row![].spacing(2).align_y(iced::Center);
        if wide && self.reword.is_none() && self.suggestion.is_none() {
            bar = bar.push(
                container(text(format!("{lines}  ·  {kind}  ·  {verdict}")).size(12).wrapping(text::Wrapping::None))
                    .padding(Padding { left: 8.0, right: 10.0, ..Padding::ZERO }),
            );
        }
        if self.reword.is_some() || self.suggestion.is_some() {
            bar = bar.push(space::horizontal());
        }
        let includes = self.includes_function(tab, hunk.key);
        let include_style = self.toggle_style(includes);
        let include = self.tip(
            button(text(if self.ctrl_held { "D" } else { "{ }" }).size(11).font(Font::with_name("Consolas")))
                .padding([5, 6])
                .style(include_style)
                .on_press(Message::ToggleIncludeFunction),
            if includes { "Include function definition: on (D)" } else { "Include function definition: off (D)" },
        );
        let suggest = button(self.labelled_with(self.sparkle(a.suggest), "A", "Suggest"))
            .padding(Padding { top: 4.0, bottom: 4.0, left: 3.0, right: 8.0 })
            .style(flat)
            .on_press_maybe(a.suggest.then_some(Message::Do(Action::Suggest)));
        let bar = bar
            .push(action(glyph::FIX, "F", fix_label, Action::Fix, a.fix, false))
            .push(action(
                glyph::IGNORE,
                "I",
                if tab.ignored.contains(&hunk.key) { "Unignore" } else { "Ignore" },
                Action::Ignore,
                a.ignore,
                false,
            ))
            .push(action(glyph::SQUASH, "S", "Squash", Action::Squash, a.squash, false))
            .push(action(glyph::DELETE, "Del", "Delete", Action::Delete, a.delete, true))
            .push(action(glyph::REWORD, self.reword_shortcut(), "Reword", Action::Reword, a.reword, false))
            .push(suggest)
            .push(include)
            .push(nav(glyph::CHEVRON_LEFT, "Previous (P)", Message::Previous))
            .push(text(format!("{}/{}", index + 1, tab.doc.hunks.len())).size(11))
            .push(nav(glyph::CHEVRON_RIGHT, "Next (N)", Message::Next))
            .push(nav(glyph::CLOSE, "Close (Esc)", Message::Deselect));

        let mut panel = column![bar].spacing(6);
        if let Some(content) = &self.reword {
            let can_apply = a.reword && !content.text().trim().is_empty();
            panel = panel.push(
                text_editor(content)
                    .id(Id::new(REWORD_EDITOR))
                    .on_action(Message::RewordEdited)
                    .key_binding(|press| {
                        let focused = press.status == text_editor::Status::Focused { is_hovered: false }
                            || press.status == text_editor::Status::Focused { is_hovered: true };
                        match press.key {
                            Key::Named(Named::Enter) if focused && press.modifiers.control() => {
                                Some(text_editor::Binding::Custom(Message::RewordApply))
                            }
                            Key::Named(Named::Escape) if focused => {
                                Some(text_editor::Binding::Custom(Message::RewordCancel))
                            }
                            _ => text_editor::Binding::from_key_press(press),
                        }
                    })
                    .font(Font::with_name("Consolas"))
                    .size(13)
                    .height(110),
            );
            let cancel = button(self.labelled(glyph::CLOSE, "Esc", "Cancel"))
                .padding([4, 10])
                .style(button::secondary)
                .on_press(Message::RewordCancel);
            let apply = button(self.labelled(glyph::REWORD, "↵", "Replace comment"))
                .padding([4, 10])
                .on_press_maybe(can_apply.then_some(Message::RewordApply));
            panel = panel.push(row![space::horizontal(), cancel, apply].spacing(6));
        }

        if let Some(suggestion) = &self.suggestion {
            panel = panel.push(self.suggestion_panel(suggestion, hunk, a));
        }

        let mut boxed = container(panel).padding([3, 4]).style(self.panel_style());
        if self.reword.is_some() || self.suggestion.is_some() {
            boxed = boxed.width((room - SCROLLBAR_ROOM * 2.0).min(REWORD_WIDTH));
        }
        boxed.into()
    }

    fn suggestion_height(&self) -> f32 {
        const CHROME: f32 = 72.0;
        match self.suggestion.as_ref().map(|s| &s.state) {
            None => 0.0,
            Some(SuggestionState::Loading) => 34.0,
            Some(SuggestionState::Failed(_)) => 66.0,
            Some(SuggestionState::Ready(suggested)) => {
                CHROME + suggestion_rows(&suggested.text).min(SUGGESTION_ROWS) as f32 * SUGGESTION_ROW_HEIGHT
            }
        }
    }

    fn suggestion_panel<'a>(&'a self, suggestion: &'a Suggestion, hunk: &Hunk, a: Available) -> Element<'a, Message> {
        let body: Element<'a, Message> = match &suggestion.state {
            SuggestionState::Loading => row![
                container(self.spinner()).padding(Padding { left: 4.0, ..Padding::ZERO }),
                text("Writing a suggestion…").size(12).width(Fill),
                button(self.labelled(glyph::CLOSE, "Esc", "Cancel"))
                    .padding(Padding { top: 4.0, bottom: 4.0, left: 3.0, right: 8.0 })
                    .style(self.flat_button(false))
                    .on_press(Message::SuggestionReject),
            ]
            .spacing(6)
            .align_y(iced::Center)
            .into(),
            SuggestionState::Failed(error) => column![
                container(text(error.as_str()).size(12)).padding(Padding { left: 4.0, ..Padding::ZERO }),
                row![
                    button(self.labelled(glyph::RETRY, "R", "Retry"))
                        .padding([4, 10])
                        .on_press(Message::SuggestionRetry),
                    button(self.labelled(glyph::CLOSE, "Esc", "Close"))
                        .padding([4, 10])
                        .style(button::secondary)
                        .on_press(Message::SuggestionReject),
                ]
                .spacing(6),
            ]
            .spacing(6)
            .into(),
            SuggestionState::Ready(suggested) => {
                let unchanged = suggested.text == hunk.raw;
                let sent = match suggested.sent {
                    Sent::CommentOnly => "Sent the comment only".to_owned(),
                    Sent::NoFunctionFound => "No function found, so only the comment was sent".to_owned(),
                    Sent::Function { first, last, partial: false } => {
                        format!("Sent the function, lines {}–{}", first + 1, last + 1)
                    }
                    Sent::Function { first, last, partial: true } => {
                        format!("Sent part of the function, lines {}–{}", first + 1, last + 1)
                    }
                };
                let caption = if unchanged { format!("{sent}  ·  Same as the current comment") } else { sent };
                let tint = mix(self.background, Color::from_rgb(0.25, 0.7, 0.35), 0.20);
                let code = container(
                    text(suggested.text.as_str())
                        .font(Font::with_name("Consolas"))
                        .size(12)
                        .shaping(text::Shaping::Advanced),
                )
                .padding([3, 8])
                .width(Fill)
                .style(move |_: &Theme| container::Style {
                    background: Some(Background::Color(tint)),
                    ..Default::default()
                });
                let rows = suggestion_rows(&suggested.text);
                let code: Element<'a, Message> = if rows > SUGGESTION_ROWS {
                    scrollable(code).height(SUGGESTION_ROWS as f32 * SUGGESTION_ROW_HEIGHT).into()
                } else {
                    code.into()
                };
                column![
                    row![
                        text("Suggested comment").size(12).width(Fill),
                        text(caption).size(11),
                    ]
                    .padding(Padding { left: 4.0, right: 4.0, ..Padding::ZERO })
                    .align_y(iced::Center),
                    code,
                    row![
                        button(self.labelled(glyph::CHECK, "↵", "Approve"))
                            .padding([4, 10])
                            .on_press_maybe((a.reword && !unchanged).then_some(Message::SuggestionApprove)),
                        button(self.labelled(glyph::CLOSE, "Esc", "Reject"))
                            .padding([4, 10])
                            .style(button::secondary)
                            .on_press(Message::SuggestionReject),
                        button(self.labelled(glyph::RETRY, "R", "Retry"))
                            .padding([4, 10])
                            .style(button::secondary)
                            .on_press(Message::SuggestionRetry),
                    ]
                    .spacing(6),
                ]
                .spacing(6)
                .into()
            }
        };
        container(body).padding(Padding { bottom: 3.0, ..Padding::ZERO }).into()
    }

    fn can_clean_folder(&self) -> bool {
        self.root.is_some() && !self.busy && !self.scan_running
    }

    fn plan_page<'a>(&'a self, plan: &'a Plan) -> Element<'a, Message> {
        let (deletes_on, deletes) = plan.count(false);
        let (squashes_on, squashes) = plan.count(true);
        let selected = deletes_on + squashes_on;
        let (title, note) = if plan.folder {
            (
                "Clean folder",
                "Nothing changes until you apply. Squashes start unticked, because fixed rules can't judge context. Undo Folder Cleanup restores every file.",
            )
        } else {
            (self.fix_mode.title(), self.fix_mode.note())
        };
        let delete_label = if !plan.folder && self.fix_mode == FixMode::Delete { "Delete" } else { "Delete junk" };
        let kind = |label: String, ticked: usize, total: usize, squash: bool| {
            checkbox(ticked == total && total > 0)
                .label(label)
                .text_size(13)
                .size(15)
                .on_toggle(move |value| Message::PlanToggleKind(squash, value))
        };
        let header = column![
            row![
                text(title).size(20).width(Fill),
                button(text("Cancel").size(12))
                    .padding([4, 12])
                    .style(button::secondary)
                    .on_press(Message::PlanCancel),
                button(text(format!("Apply ({selected})")).size(12))
                    .padding([4, 12])
                    .on_press_maybe((selected > 0).then_some(Message::PlanApply)),
            ]
            .spacing(8)
            .align_y(iced::Center),
            text(note).size(12),
            row![
                kind(format!("{delete_label} ({deletes_on} of {deletes})"), deletes_on, deletes, false),
                kind(format!("Squash ({squashes_on} of {squashes})"), squashes_on, squashes, true),
            ]
            .spacing(24),
        ]
        .spacing(10)
        .padding(Padding { top: 16.0, right: 20.0, bottom: 12.0, left: 20.0 });
        let uncommitted = plan.files.iter().filter(|file| self.git_state(&file.path).is_some()).count();
        let header = if uncommitted > 0 {
            let warning = match (plan.folder, uncommitted) {
                (false, _) => "This file has uncommitted changes. Commit first to keep this cleanup separate from your own edits.".to_owned(),
                (true, 1) => "1 of these files has uncommitted changes. Commit first to keep this cleanup separate from your own edits.".to_owned(),
                (true, n) => format!("{n} of these files have uncommitted changes. Commit first to keep this cleanup separate from your own edits."),
            };
            header.push(text(warning).size(12).color(git_color(FileState::Modified)))
        } else {
            header
        };
        let header = if self.repo.is_some() {
            header.push(
                checkbox(self.only_changed)
                    .label("Only comments on lines changed since the last commit")
                    .text_size(13)
                    .size(15)
                    .on_toggle(|_| Message::ToggleOnlyChanged),
            )
        } else {
            header
        };
        let header = if plan.files.is_empty() {
            header.push(text("Nothing to clean on the lines you've changed.").size(13))
        } else {
            header
        };

        let rows = plan.files.iter().enumerate().map(|(index, file)| self.plan_file(plan.folder, index, file));
        let list = column(rows)
            .spacing(2)
            .padding(Padding { top: 0.0, right: 20.0, bottom: 20.0, left: 20.0 });
        column![header, scrollable(list).width(Fill).height(Fill)].into()
    }

    fn plan_file<'a>(&'a self, folder: bool, index: usize, file: &'a PlanFile) -> Element<'a, Message> {
        let ticked = file.items.iter().filter(|item| item.include).count();
        let chevron = if file.expanded { glyph::CHEVRON_DOWN } else { glyph::CHEVRON_RIGHT };
        let mut section = column![].spacing(6);
        if folder {
            section = section.push(
                row![
                    checkbox(ticked == file.items.len())
                        .size(15)
                        .on_toggle(move |value| Message::PlanToggleFile(index, value)),
                    button(
                        row![
                            symbol(chevron, 8.0),
                            text(file.label.as_str())
                                .size(13)
                                .width(Fill)
                                .shaping(text::Shaping::Advanced)
                                .wrapping(text::Wrapping::None),
                            text(self.git_state(&file.path).map_or("", FileState::letter))
                                .size(11)
                                .color(self.git_state(&file.path).map_or(self.palette.foreground, git_color)),
                            text(format!("{ticked} of {}", file.items.len())).size(12),
                        ]
                        .spacing(8)
                        .align_y(iced::Center),
                    )
                    .width(Fill)
                    .padding([3, 6])
                    .style(self.flat_button(false))
                    .on_press(Message::PlanExpand(index)),
                ]
                .spacing(4)
                .align_y(iced::Center),
            );
        }
        if file.expanded {
            let shown = file.items.iter().enumerate().take(MAX_PREVIEW_ITEMS);
            for (item_index, item) in shown {
                section = section.push(self.plan_change(index, item_index, item));
            }
            if file.items.len() > MAX_PREVIEW_ITEMS {
                section = section.push(
                    text(format!(
                        "{} more not listed. They follow this file's checkbox.",
                        file.items.len() - MAX_PREVIEW_ITEMS
                    ))
                    .size(12),
                );
            }
        }
        section.into()
    }

    fn plan_change<'a>(&'a self, file: usize, index: usize, item: &'a PlanItem) -> Element<'a, Message> {
        let code = |content: &'a str, tint: Color| {
            let background = mix(self.background, tint, 0.20);
            container(
                text(content)
                    .font(Font::with_name("Consolas"))
                    .size(12)
                    .shaping(text::Shaping::Advanced),
            )
            .padding([3, 8])
            .width(Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(background)),
                ..Default::default()
            })
        };
        let mut change = column![
            row![
                checkbox(item.include)
                    .size(14)
                    .on_toggle(move |value| Message::PlanToggleItem(file, index, value)),
                text(format!("Line {}  ·  {}", item.hunk.first + 1, item.reason)).size(12),
            ]
            .spacing(4)
            .align_y(iced::Center),
            code(item.hunk.raw.as_str(), Color::from_rgb(0.85, 0.25, 0.25)),
        ]
        .spacing(3);
        if let Change::Replace(replacement) = &item.change {
            change = change.push(code(replacement.as_str(), Color::from_rgb(0.25, 0.7, 0.35)));
        }
        container(change).padding(Padding { left: 22.0, top: 4.0, bottom: 6.0, ..Padding::ZERO }).into()
    }

    fn settings_page(&self) -> Element<'_, Message> {
        let s = &self.settings;
        let toggle = |label: &'static str, value: bool, field: BoolField| {
            checkbox(value)
                .label(label)
                .text_size(13)
                .size(15)
                .on_toggle(move |v| Message::SetBool(field, v))
        };
        let heading = |label: &'static str, note: &'static str| {
            column![text(label).size(15), text(note).size(12)].spacing(2)
        };
        let patterns = |content, on_edit: fn(text_editor::Action) -> Message| {
            text_editor(content)
                .on_action(on_edit)
                .placeholder("One regular expression per line")
                .font(Font::with_name("Consolas"))
                .size(13)
                .height(64)
        };
        let number = |label: &'static str, value, on_input: fn(String) -> Message| {
            row![
                text(label).size(13).width(240),
                text_input("", value).on_input(on_input).size(13).padding([3, 6]).width(80),
            ]
            .align_y(iced::Center)
        };

        let themes = self
            .highlighter
            .as_ref()
            .map(|h| h.theme_names.as_slice())
            .unwrap_or(&[]);
        let appearance = column![
            heading("Appearance", "Applies to every folder."),
            row![
                text("Color theme").size(13).width(240),
                pick_list(themes, Some(&self.theme_name), Message::ThemePicked)
                    .text_size(13)
                    .padding([3, 8])
                    .width(280),
            ]
            .align_y(iced::Center),
            row![
                text("File icons").size(13).width(240),
                pick_list(
                    self.icon_theme_names.as_slice(),
                    Some(&self.icon_theme_name),
                    Message::IconThemePicked
                )
                .text_size(13)
                .padding([3, 8])
                .width(280),
            ]
            .align_y(iced::Center),
        ]
        .spacing(8);

        let field = |label: &'static str| text(label).size(13).width(240);
        let provider = self.ai.provider;
        let key_placeholder = if self.ai_has_key {
            "Saved. Paste a new key to replace it.".to_owned()
        } else if provider.requires_key() {
            format!("Paste your {} API key", provider.name())
        } else {
            "Only if your server needs one".to_owned()
        };
        let mut key_row = row![
            field("API key"),
            text_input(&key_placeholder, &self.ai_key_input)
                .secure(true)
                .on_input(Message::AiKeyInput)
                .on_submit(Message::AiKeySave)
                .size(13)
                .padding([3, 6])
                .width(280),
            button(text("Save").size(12))
                .padding([3, 12])
                .on_press_maybe((!self.ai_key_input.trim().is_empty()).then_some(Message::AiKeySave)),
        ]
        .spacing(6)
        .align_y(iced::Center);
        if self.ai_has_key {
            key_row = key_row.push(
                button(text("Remove").size(12))
                    .padding([3, 12])
                    .style(button::secondary)
                    .on_press(Message::AiKeyRemove),
            );
        }
        let key_note = match &self.ai_models_error {
            Some(error) => error.as_str(),
            None if provider == Provider::Ollama => "Runs on this computer, so no key is needed. Start Ollama, then choose a model.",
            None if self.ai_has_key => "Encrypted for your Windows account and stored apart from your settings.",
            None => "Encrypted for your Windows account when saved.",
        };
        let model_placeholder = if provider.default_model().is_empty() { "Model name" } else { provider.default_model() };
        let mut model_row = row![
            field("Model"),
            text_input(model_placeholder, &self.ai.model)
                .on_input(Message::AiModelInput)
                .size(13)
                .padding([3, 6])
                .width(280),
        ]
        .spacing(6)
        .align_y(iced::Center);
        if !self.ai_models.is_empty() {
            let selected = self.ai_models.iter().find(|model| **model == self.ai.model);
            model_row = model_row.push(
                pick_list(self.ai_models.as_slice(), selected, Message::AiModelInput)
                    .placeholder("Choose…")
                    .text_size(13)
                    .padding([3, 8])
                    .width(200),
            );
        }
        let provider_row = row![
            field("Provider"),
            pick_list(&Provider::ALL[..], Some(provider), Message::AiProviderPicked)
                .text_size(13)
                .padding([3, 8])
                .width(280),
        ]
        .spacing(6)
        .align_y(iced::Center);
        let mut connection = column![provider_row].spacing(8);
        if provider == Provider::Custom {
            connection = connection.push(
                row![
                    field("Server address"),
                    text_input("https://example.com/v1", &self.ai.custom_url)
                        .on_input(Message::AiCustomUrlInput)
                        .on_submit(Message::AiCustomUrlSubmit)
                        .size(13)
                        .padding([3, 6])
                        .width(280),
                ]
                .spacing(6)
                .align_y(iced::Center),
            );
        }
        if provider.takes_key() {
            connection = connection.push(key_row);
        }
        let mut suggestions = column![
            heading("AI suggestions", "Applies to every folder."),
            connection,
            row![space::horizontal().width(246), text(key_note).size(12)],
            model_row,
            row![
                field("Preset"),
                pick_list(&Preset::ALL[..], Some(self.ai.preset), Message::AiPresetPicked)
                    .text_size(13)
                    .padding([3, 8])
                    .width(280),
            ]
            .spacing(6)
            .align_y(iced::Center),
            row![space::horizontal().width(246), text(self.ai.preset.description()).size(12)],
        ]
        .spacing(8);
        if self.ai.preset == Preset::Custom {
            suggestions = suggestions.push(text("Suggestion Instructions").size(13)).push(
                text_editor(&self.ai_custom)
                    .on_action(Message::AiCustomEdited)
                    .placeholder("How should comments be written?")
                    .size(13)
                    .height(96),
            );
        }
        let suggestions = suggestions.push(
            column![
                checkbox(self.ai.include_function)
                    .label("Include function definition")
                    .text_size(13)
                    .size(15)
                    .on_toggle(Message::AiIncludeFunction),
                text(format!("The default for each comment; {{ }} on the toolbar changes it for one comment. Suggest sends the comment and its language to {}, plus the whole function when this is on.", provider.name())).size(12),
            ]
            .spacing(4),
        );
        let suggestions = suggestions.push(
            column![
                checkbox(self.ai.suggest_ahead)
                    .label("Suggest ahead while going through all")
                    .text_size(13)
                    .size(15)
                    .on_toggle(Message::AiSuggestAhead),
                text(format!("Each comment opens with a suggestion ready: Enter approves, Esc keeps the comment and moves on. Sends the next few comments to {} as you go.", provider.name())).size(12),
            ]
            .spacing(4),
        );

        let protect = column![
            heading("Protect", "Fix and Fix all leave these alone, and Go through all skips them."),
            toggle("Tool directives (eslint-disable, @ts-ignore, noqa, go:build, shebangs…)", s.protect_directives, |s| &mut s.protect_directives),
            toggle("Licence and copyright notices", s.protect_licenses, |s| &mut s.protect_licenses),
            toggle("TODO, FIXME, HACK, XXX and BUG notes", s.protect_todos, |s| &mut s.protect_todos),
            text("Also protect comments matching:").size(13),
            patterns(&self.protect_patterns, Message::ProtectEdited),
        ]
        .spacing(8);

        let delete = column![
            heading("Fix deletes a comment when", "Checked before squashing."),
            toggle("It has no text", s.delete_empty, |s| &mut s.delete_empty),
            toggle("It is only a divider, such as // -------", s.delete_dividers, |s| &mut s.delete_dividers),
            toggle("Every line looks like commented-out code (a best guess)", s.delete_commented_code, |s| &mut s.delete_commented_code),
            text("Also delete comments matching:").size(13),
            patterns(&self.delete_patterns, Message::DeleteEdited),
        ]
        .spacing(8);

        let squash = column![
            heading("Fix squashes a comment when", "It spans several lines and fits these limits."),
            toggle("Squash comments", s.squash_enabled, |s| &mut s.squash_enabled),
            number("Result fits within (columns)", self.width_input.as_str(), Message::SetWidth),
            number("Comment spans at most (lines)", self.lines_input.as_str(), Message::SetLines),
            toggle("Include doc comments (///, /** */)", s.squash_doc_comments, |s| &mut s.squash_doc_comments),
            toggle("Include bulleted or numbered lists", s.squash_lists, |s| &mut s.squash_lists),
        ]
        .spacing(8);

        let mut page = column![text("Settings").size(20), appearance, suggestions, self.updates_section()]
            .spacing(22)
            .padding(20)
            .max_width(720);
        if self.root.is_some() {
            let note = if self.folder_settings {
                "Saved in .commentcleanup.json in this folder. Untick to go back to the app-wide settings."
            } else {
                "The rules below apply to every folder."
            };
            page = page.push(
                column![
                    checkbox(self.folder_settings)
                        .label("Use separate settings for this folder")
                        .text_size(13)
                        .size(15)
                        .on_toggle(Message::FolderSettings),
                    text(note).size(12),
                ]
                .spacing(4),
            );
        }
        page = page.push(protect).push(delete).push(squash).push(self.rule_test_section());
        let page = page.padding(Padding { top: if self.rules_dirty() { 52.0 } else { 20.0 }, ..Padding::new(20.0) });
        stack![scrollable(page).width(Fill).height(Fill), self.rules_bar()].width(Fill).height(Fill).into()
    }

    fn rules_bar(&self) -> Element<'_, Message> {
        if !self.rules_dirty() {
            return space::vertical().height(0).into();
        }
        let summary = if !self.edited_rules.errors.is_empty() {
            format!("Fix before saving, not a valid pattern: {}", self.edited_rules.errors.join(", "))
        } else {
            match &self.rule_impact {
                Some(impact) => match impact.differences.len() + impact.more {
                    0 => "Unsaved rule changes. No comments would act differently.".to_owned(),
                    1 => "Unsaved rule changes. 1 comment would act differently.".to_owned(),
                    n => format!("Unsaved rule changes. {n} comments would act differently."),
                },
                None => "Unsaved rule changes.".to_owned(),
            }
        };
        let bar = container(
            row![
                text(summary).size(12).width(Fill).shaping(text::Shaping::Advanced),
                button(text("Discard").size(12))
                    .padding([3, 12])
                    .style(button::secondary)
                    .on_press(Message::DiscardRules),
                button(text("Save rules").size(12))
                    .padding([3, 12])
                    .on_press_maybe(self.can_save_rules().then_some(Message::SaveRules)),
            ]
            .spacing(8)
            .align_y(iced::Center),
        )
        .padding([6, 20])
        .width(Fill)
        .style(self.background_style(0.10));
        opaque(bar)
    }

    fn rule_test_section(&self) -> Element<'_, Message> {
        let mut section = column![
            column![
                text("Test your rules").size(15),
                text("Changes above stay here until you save them, and testing never touches a file.").size(12),
            ]
            .spacing(2),
            text("Try a comment:").size(13),
            text_editor(&self.rule_sample)
                .on_action(Message::RuleSampleEdited)
                .placeholder("// Paste a comment to see what the rules would do")
                .font(Font::with_name("Consolas"))
                .size(13)
                .height(64),
        ]
        .spacing(8);
        if let Some(result) = &self.rule_sample_result {
            section = section.push(text(result.as_str()).size(13).shaping(text::Shaping::Advanced));
        }
        let Some(impact) = &self.rule_impact else {
            return section.into();
        };
        let scope = if self.root.is_some() { "this folder" } else { "your open files" };
        let reading = if self.scan_running { " (still reading the folder)" } else { "" };
        let dirty = self.rules_dirty();
        let tally = |label: &str, pick: fn(&Tally) -> usize| {
            let (now, saved) = (pick(&impact.now), pick(&impact.saved));
            let change = if dirty && now != saved {
                format!(" ({}{})", if now > saved { "+" } else { "−" }, now.abs_diff(saved))
            } else {
                String::new()
            };
            format!("{label} {now}{change}")
        };
        section = section
            .push(text(format!("Across {scope}{reading}:")).size(13))
            .push(
                text(format!(
                    "{}   ·   {}   ·   {}   ·   {}",
                    tally("Delete", |t| t.delete),
                    tally("Squash", |t| t.squash),
                    tally("Protect", |t| t.protect),
                    tally("Leave alone", |t| t.keep),
                ))
                .size(13),
            );
        if dirty && !impact.differences.is_empty() {
            section = section.push(text("Comments that would act differently:").size(13));
            let dim = Color { a: 0.7, ..self.palette.foreground };
            for difference in &impact.differences {
                section = section.push(
                    column![
                        text(format!(
                            "{}:{}   {} → {}",
                            difference.label, difference.line, difference.before, difference.after
                        ))
                        .size(12)
                        .color(dim)
                        .shaping(text::Shaping::Advanced),
                        text(difference.comment.as_str())
                            .size(12)
                            .font(Font::with_name("Consolas"))
                            .shaping(text::Shaping::Advanced),
                    ]
                    .spacing(1),
                );
            }
            if impact.more > 0 {
                section = section.push(text(format!("And {} more.", impact.more)).size(12));
            }
        }
        section.into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let details = self.tab().map(|tab| {
            let note = tab.read_only.as_deref().unwrap_or("");
            let git = self.repo.as_ref().and_then(|repo| repo.files.get(&tab.path)).map_or(String::new(), |file| {
                match file.changed.line_count() {
                    Some(lines) => format!("{}, {lines} lines", file.state.describe()),
                    None => file.state.describe().to_owned(),
                }
            });
            format!(
                "{note}   {git}   {}   {} lines   {} comments",
                tab.doc.language,
                tab.doc.lines.len(),
                tab.doc.hunks.len()
            )
        });
        container(
            row![
                text(&self.status).size(12).width(Fill),
                text(if self.scan_running {
                    format!("Reading comments… {} files", self.scan.len())
                } else {
                    String::new()
                })
                .size(12),
                button(self.labelled(glyph::UNDO, "Z", "Undo"))
                    .padding(Padding { top: 1.0, bottom: 1.0, left: 2.0, right: 8.0 })
                    .style(self.flat_button(false))
                    .on_press_maybe(self.can_undo().then_some(Message::Undo)),
                text(details.unwrap_or_default()).size(12),
            ]
            .spacing(12)
            .align_y(iced::Center),
        )
        .padding([3, 10])
        .width(Fill)
        .style(self.background_style(0.04))
        .into()
    }
}

fn plan_item(hunk: &Hunk, decision: &Decision, squash_ticked: bool) -> Option<PlanItem> {
    let (change, reason, include) = match decision {
        Decision::Squash(text) => (Change::Replace(text.clone()), "Squash into one line".to_owned(), squash_ticked),
        Decision::Delete(why) => (Change::Delete, format!("Delete: {why}"), true),
        _ => return None,
    };
    Some(PlanItem { hunk: hunk.clone(), change, reason, include })
}

fn suggestion_rows(text: &str) -> usize {
    text.lines().map(|line| line.chars().count().div_ceil(SUGGESTION_ROW_CHARS).max(1)).sum()
}

fn in_scope(scope: Option<Option<&Changed>>, hunk: &Hunk) -> bool {
    scope.is_none_or(|changed| changed.is_some_and(|changed| changed.touches(hunk.first, hunk.last)))
}

fn is_git_internal(root: Option<&Path>, path: &Path) -> bool {
    root.and_then(|root| path.strip_prefix(root).ok())
        .and_then(|relative| relative.components().next())
        .is_some_and(|first| first.as_os_str() == ".git")
}

fn is_git_state(path: &Path) -> bool {
    let name = path.file_name().map(|name| name.to_string_lossy()).unwrap_or_default();
    !name.ends_with(".lock")
        && (matches!(name.as_ref(), "index" | "HEAD" | "ORIG_HEAD" | "MERGE_HEAD")
            || path.components().any(|part| part.as_os_str() == "refs"))
}

fn git_color(state: FileState) -> Color {
    match state {
        FileState::Modified => Color::from_rgb8(0xe2, 0xc0, 0x8d),
        FileState::Added | FileState::Untracked => Color::from_rgb8(0x73, 0xc9, 0x91),
        FileState::Conflict => Color::from_rgb8(0xe4, 0x67, 0x6b),
    }
}

fn symbol<'a>(glyph: char, size: f32) -> iced::widget::Text<'a> {
    text(glyph.to_string())
        .font(icons::SYMBOLS)
        .size(size)
        .shaping(text::Shaping::Advanced)
}

fn pattern_lines(content: &text_editor::Content) -> Vec<String> {
    content
        .text()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn report(label: &str, values: &mut Vec<f32>) {
    if values.is_empty() {
        println!("{label}: no samples");
        return;
    }
    values.sort_by(f32::total_cmp);
    let at = |p: f32| values[((values.len() - 1) as f32 * p) as usize];
    println!(
        "{label}: n={} p50={:.2} p95={:.2} p99={:.2} max={:.2}",
        values.len(),
        at(0.5),
        at(0.95),
        at(0.99),
        at(1.0)
    );
}

fn save_png(path: &Path, shot: &window::Screenshot) {
    let file = std::fs::File::create(path).expect("create png");
    let mut encoder = png::Encoder::new(file, shot.size.width, shot.size.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("png header");
    writer.write_image_data(&shot.rgba).expect("png data");
}

fn watch_path(target: &(PathBuf, bool)) -> impl Stream<Item = Message> + use<> {
    let (root, recursive) = target.clone();
    let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    iced::stream::channel(8, async move |mut output: mpsc::Sender<Message>| {
        let (ui_tx, mut ui_rx) = mpsc::unbounded::<Vec<PathBuf>>();
        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            let Ok(mut watcher) = notify::recommended_watcher(tx) else {
                return;
            };
            if watcher.watch(&root, mode).is_err() {
                return;
            }
            let relevant = |event: notify::Result<notify::Event>| {
                event
                    .ok()
                    .filter(|e| !matches!(e.kind, EventKind::Access(_)))
                    .map(|e| e.paths)
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|p| !p.to_string_lossy().ends_with(files::TEMP_SUFFIX))
            };
            while let Ok(event) = rx.recv() {
                let mut paths: Vec<PathBuf> = relevant(event).collect();
                let window = Instant::now();
                while window.elapsed() < Duration::from_millis(400) {
                    match rx.recv_timeout(Duration::from_millis(30)) {
                        Ok(event) => paths.extend(relevant(event)),
                        Err(_) => break,
                    }
                }
                paths.sort();
                paths.dedup();
                if !paths.is_empty() && ui_tx.unbounded_send(paths).is_err() {
                    break;
                }
            }
        });
        while let Some(paths) = ui_rx.next().await {
            let _ = output.send(Message::FilesChanged(paths)).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::tests::highlighter;
    use iced::futures::executor::block_on;

    /// What each mode would do to a file, as (first line, is a delete).
    fn plan_for(mode: FixMode, source: &str) -> Vec<(usize, bool)> {
        let h = highlighter();
        let doc = h.highlight(source, "javascript", "Dark+").unwrap();
        let rules = Rules::new(&Settings::default());
        let syntax = h.syntax_for("javascript");
        doc.hunks
            .iter()
            .filter_map(|hunk| mode.item(hunk, &rules.decide(hunk, syntax), syntax))
            .map(|item| (item.hunk.first, matches!(item.change, Change::Delete)))
            .collect()
    }

    const SOURCE: &str = "// keep me\nlet a;\n// one\n// two\nlet b;\n// -----\nlet c;\n// TODO: stays\n";

    #[test]
    fn auto_fix_follows_the_rules() {
        assert_eq!(plan_for(FixMode::Auto, SOURCE), [(2, false), (5, true)]);
    }

    #[test]
    fn delete_takes_every_comment_except_protected_ones() {
        assert_eq!(plan_for(FixMode::Delete, "// keep me\nlet a;\n// one\n// two\nlet b;\n// TODO: stays\n"), [(0, true), (2, true)]);
    }

    #[test]
    fn squash_takes_only_comments_that_span_several_lines() {
        let source = "// single\nlet a;\n// one\n// two\nlet b;\n// TODO: one\n// TODO: two\n";
        assert_eq!(plan_for(FixMode::Squash, source), [(2, false)]);
    }

    #[test]
    fn background_work_does_not_hold_up_its_caller() {
        let started = Instant::now();
        let result = spawn_work(|| {
            std::thread::sleep(Duration::from_millis(300));
            7
        });
        assert!(started.elapsed() < Duration::from_millis(150), "the caller had to wait");
        assert_eq!(block_on(result), Ok(7));
    }
}
