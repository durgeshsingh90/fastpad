use anyhow::{bail, Context, Result};
use fastpad_edit::EditBuffer;
use fastpad_file::{atomic_write, FileHandle, FileKind, FileMetadata, FileOpenOptions};
use fastpad_search::{SearchEngine, SearchQuery, SearchSummary};
use fastpad_tasks::CancellationToken;
use fastpad_viewport::{ViewAnchor, Viewport, ViewportEngine, ViewportRequest};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const DEFAULT_LARGE_FILE_BYTES: u64 = 250 * 1024 * 1024;
pub const DEFAULT_VERY_LARGE_FILE_BYTES: u64 = 1024 * 1024 * 1024;
pub const DEFAULT_HUGE_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const DEFAULT_ANALYSIS_THRESHOLD_BYTES: u64 = DEFAULT_HUGE_FILE_BYTES;
pub const DEFAULT_HUGE_EDIT_WARNING_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_MAX_EDIT_LOAD_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_HUGE_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EditorMode {
    ViewAnalysis,
    Edit,
}

impl EditorMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ViewAnalysis => "View/Analysis Mode",
            Self::Edit => "Edit Mode",
        }
    }

    pub fn is_editable(self) -> bool {
        matches!(self, Self::Edit)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub analysis_threshold_bytes: u64,
    pub large_file_bytes: u64,
    pub very_large_file_bytes: u64,
    pub huge_file_bytes: u64,
    pub huge_edit_warning_bytes: u64,
    pub max_edit_load_bytes: u64,
    pub huge_line_bytes: usize,
    pub initial_viewport_lines: usize,
    pub initial_viewport_bytes: usize,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            analysis_threshold_bytes: DEFAULT_ANALYSIS_THRESHOLD_BYTES,
            large_file_bytes: DEFAULT_LARGE_FILE_BYTES,
            very_large_file_bytes: DEFAULT_VERY_LARGE_FILE_BYTES,
            huge_file_bytes: DEFAULT_HUGE_FILE_BYTES,
            huge_edit_warning_bytes: DEFAULT_HUGE_EDIT_WARNING_BYTES,
            max_edit_load_bytes: DEFAULT_MAX_EDIT_LOAD_BYTES,
            huge_line_bytes: DEFAULT_HUGE_LINE_BYTES,
            initial_viewport_lines: 120,
            initial_viewport_bytes: 512 * 1024,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct OpenIntent {
    pub force_analysis: bool,
    pub force_edit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InternalEngineProfile {
    NormalEdit,
    LargeOptimizedEdit,
    StreamingEdit,
    HugeFileAnalysis,
    StructuredDataAnalysis,
    BinaryInspection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryPressure {
    Unknown,
    Normal,
    Elevated,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemMemoryStatus {
    pub available_bytes: Option<u64>,
    pub total_bytes: Option<u64>,
    pub pressure: MemoryPressure,
}

impl SystemMemoryStatus {
    pub fn current() -> Self {
        current_system_memory_status()
    }

    pub fn unconstrained_for_tests() -> Self {
        Self {
            available_bytes: None,
            total_bytes: None,
            pressure: MemoryPressure::Normal,
        }
    }
}

#[cfg(target_os = "macos")]
fn current_system_memory_status() -> SystemMemoryStatus {
    use std::ptr::addr_of;

    unsafe {
        let mut stats = std::mem::MaybeUninit::<libc::vm_statistics64>::zeroed().assume_init();
        let mut count = libc::HOST_VM_INFO64_COUNT;
        #[allow(deprecated)]
        let host = libc::mach_host_self();
        let result = libc::host_statistics64(
            host,
            libc::HOST_VM_INFO64,
            &mut stats as *mut libc::vm_statistics64 as libc::host_info64_t,
            &mut count,
        );
        let total_bytes = system_total_memory_bytes();
        if result != 0 {
            return SystemMemoryStatus {
                available_bytes: None,
                total_bytes,
                pressure: MemoryPressure::Unknown,
            };
        }

        let page_size = system_page_size().max(1);
        let free = addr_of!(stats.free_count).read_unaligned() as u64;
        let inactive = addr_of!(stats.inactive_count).read_unaligned() as u64;
        let speculative = addr_of!(stats.speculative_count).read_unaligned() as u64;
        let compressor = addr_of!(stats.compressor_page_count).read_unaligned() as u64;
        let available_bytes = free
            .saturating_add(inactive)
            .saturating_add(speculative)
            .saturating_mul(page_size);
        let pressure = match total_bytes {
            Some(total) if total > 0 => {
                let available_ratio = available_bytes as f64 / total as f64;
                let compressed_ratio = compressor.saturating_mul(page_size) as f64 / total as f64;
                if available_ratio < 0.05 || compressed_ratio > 0.25 {
                    MemoryPressure::Critical
                } else if available_ratio < 0.15 || compressed_ratio > 0.15 {
                    MemoryPressure::Elevated
                } else {
                    MemoryPressure::Normal
                }
            }
            _ => MemoryPressure::Unknown,
        };

        SystemMemoryStatus {
            available_bytes: Some(available_bytes),
            total_bytes,
            pressure,
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn current_system_memory_status() -> SystemMemoryStatus {
    SystemMemoryStatus {
        available_bytes: None,
        total_bytes: None,
        pressure: MemoryPressure::Unknown,
    }
}

#[cfg(target_os = "macos")]
fn system_page_size() -> u64 {
    unsafe {
        let page_size = libc::sysconf(libc::_SC_PAGE_SIZE);
        if page_size > 0 {
            page_size as u64
        } else {
            libc::vm_page_size as u64
        }
    }
}

#[cfg(target_os = "macos")]
fn system_total_memory_bytes() -> Option<u64> {
    unsafe {
        let pages = libc::sysconf(libc::_SC_PHYS_PAGES);
        let page_size = system_page_size();
        if pages > 0 && page_size > 0 {
            Some((pages as u64).saturating_mul(page_size))
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeDecision {
    pub mode: EditorMode,
    pub internal_engine: InternalEngineProfile,
    pub reason: String,
    pub user_notice: Option<String>,
    pub requires_huge_edit_warning: bool,
}

pub struct ModeManager {
    settings: AppSettings,
}

impl ModeManager {
    pub fn new(settings: AppSettings) -> Self {
        Self { settings }
    }

    pub fn choose_for_open(&self, metadata: &FileMetadata, intent: OpenIntent) -> ModeDecision {
        self.choose_for_open_with_system(metadata, intent, SystemMemoryStatus::current())
    }

    pub fn choose_for_open_with_system(
        &self,
        metadata: &FileMetadata,
        intent: OpenIntent,
        system: SystemMemoryStatus,
    ) -> ModeDecision {
        if intent.force_analysis {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: self.analysis_profile(metadata),
                reason: "forced read-only analysis".into(),
                user_notice: None,
                requires_huge_edit_warning: false,
            };
        }

        if intent.force_edit {
            let internal_engine = self.edit_profile(metadata);
            return ModeDecision {
                mode: EditorMode::Edit,
                internal_engine,
                reason: "forced edit".into(),
                user_notice: optimization_notice(internal_engine),
                requires_huge_edit_warning: metadata.len >= self.settings.huge_edit_warning_bytes,
            };
        }

        if !metadata.intelligence.likely_text || matches!(metadata.kind, FileKind::Binary) {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: InternalEngineProfile::BinaryInspection,
                reason: "binary-like content opened using bounded inspection path".into(),
                user_notice: None,
                requires_huge_edit_warning: false,
            };
        }

        if metadata.intelligence.longest_line_bytes >= self.settings.huge_line_bytes {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: InternalEngineProfile::HugeFileAnalysis,
                reason: "extremely long line requires viewport-driven analysis".into(),
                user_notice: Some("Large File Optimizations Enabled".into()),
                requires_huge_edit_warning: false,
            };
        }

        if self.is_structured_analysis_candidate(metadata) {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: InternalEngineProfile::StructuredDataAnalysis,
                reason: "large structured data file opened with streaming analysis".into(),
                user_notice: None,
                requires_huge_edit_warning: false,
            };
        }

        if self.exceeds_huge_threshold(metadata) {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: InternalEngineProfile::HugeFileAnalysis,
                reason: "huge file opened using bounded read-only analysis".into(),
                user_notice: None,
                requires_huge_edit_warning: false,
            };
        }

        if self.memory_pressure_requires_analysis(metadata, system) {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: self.analysis_profile(metadata),
                reason: "current memory constraints require bounded analysis".into(),
                user_notice: Some("Large File Optimizations Enabled".into()),
                requires_huge_edit_warning: false,
            };
        }

        if metadata.len > self.settings.max_edit_load_bytes {
            return ModeDecision {
                mode: EditorMode::ViewAnalysis,
                internal_engine: InternalEngineProfile::HugeFileAnalysis,
                reason: "file exceeds currently bounded edit-engine load".into(),
                user_notice: Some("Large File Optimizations Enabled".into()),
                requires_huge_edit_warning: false,
            };
        }

        let internal_engine = self.edit_profile(metadata);
        ModeDecision {
            mode: EditorMode::Edit,
            internal_engine,
            reason: self.edit_reason(metadata, internal_engine),
            user_notice: optimization_notice(internal_engine),
            requires_huge_edit_warning: false,
        }
    }

    fn exceeds_huge_threshold(&self, metadata: &FileMetadata) -> bool {
        let huge_threshold = self
            .settings
            .analysis_threshold_bytes
            .min(self.settings.huge_file_bytes);
        metadata.len >= huge_threshold
    }

    fn is_structured_analysis_candidate(&self, metadata: &FileMetadata) -> bool {
        matches!(
            metadata.kind,
            FileKind::Csv | FileKind::Tsv | FileKind::SqlDump
        ) && metadata.len >= self.settings.large_file_bytes
    }

    fn memory_pressure_requires_analysis(
        &self,
        metadata: &FileMetadata,
        system: SystemMemoryStatus,
    ) -> bool {
        if metadata.len < self.settings.large_file_bytes {
            return false;
        }
        if matches!(system.pressure, MemoryPressure::Critical) {
            return true;
        }
        if let Some(available) = system.available_bytes {
            return metadata.len.saturating_mul(3) > available;
        }
        if let Some(total) = system.total_bytes {
            return metadata.len.saturating_mul(2) > total;
        }
        false
    }

    fn analysis_profile(&self, metadata: &FileMetadata) -> InternalEngineProfile {
        if matches!(metadata.kind, FileKind::Binary) || !metadata.intelligence.likely_text {
            InternalEngineProfile::BinaryInspection
        } else if matches!(
            metadata.kind,
            FileKind::Csv | FileKind::Tsv | FileKind::SqlDump
        ) {
            InternalEngineProfile::StructuredDataAnalysis
        } else {
            InternalEngineProfile::HugeFileAnalysis
        }
    }

    fn edit_profile(&self, metadata: &FileMetadata) -> InternalEngineProfile {
        if metadata.len >= self.settings.very_large_file_bytes {
            InternalEngineProfile::StreamingEdit
        } else if metadata.len >= self.settings.large_file_bytes
            || metadata.intelligence.very_long_line_warning
            || metadata.intelligence.average_line_bytes >= 8 * 1024
        {
            InternalEngineProfile::LargeOptimizedEdit
        } else {
            InternalEngineProfile::NormalEdit
        }
    }

    fn edit_reason(&self, metadata: &FileMetadata, profile: InternalEngineProfile) -> String {
        match profile {
            InternalEngineProfile::NormalEdit => "file fits normal edit path".into(),
            InternalEngineProfile::LargeOptimizedEdit => {
                if metadata.intelligence.very_long_line_warning {
                    "long lines require lazy measurement and virtual rendering".into()
                } else {
                    "large file uses edit mode with optimizations".into()
                }
            }
            InternalEngineProfile::StreamingEdit => {
                "very large file uses streaming edit optimizations".into()
            }
            InternalEngineProfile::HugeFileAnalysis
            | InternalEngineProfile::StructuredDataAnalysis
            | InternalEngineProfile::BinaryInspection => "analysis profile selected".into(),
        }
    }
}

fn optimization_notice(profile: InternalEngineProfile) -> Option<String> {
    match profile {
        InternalEngineProfile::LargeOptimizedEdit | InternalEngineProfile::StreamingEdit => {
            Some("Large File Optimizations Enabled".into())
        }
        InternalEngineProfile::NormalEdit
        | InternalEngineProfile::HugeFileAnalysis
        | InternalEngineProfile::StructuredDataAnalysis
        | InternalEngineProfile::BinaryInspection => None,
    }
}

pub enum DocumentBacking {
    View {
        file: FileHandle,
        viewport: ViewportEngine,
    },
    Edit {
        buffer: EditBuffer,
        path: Option<PathBuf>,
        metadata: Option<FileMetadata>,
    },
}

pub struct Document {
    id: DocumentId,
    title: String,
    mode: EditorMode,
    internal_engine: InternalEngineProfile,
    mode_reason: String,
    user_notice: Option<String>,
    backing: DocumentBacking,
}

impl Document {
    pub fn untitled(id: DocumentId) -> Self {
        Self {
            id,
            title: "Untitled".into(),
            mode: EditorMode::Edit,
            internal_engine: InternalEngineProfile::NormalEdit,
            mode_reason: "new document".into(),
            user_notice: None,
            backing: DocumentBacking::Edit {
                buffer: EditBuffer::new(),
                path: None,
                metadata: None,
            },
        }
    }

    pub fn open(
        id: DocumentId,
        path: impl AsRef<Path>,
        settings: &AppSettings,
        intent: OpenIntent,
    ) -> Result<Self> {
        let file = FileHandle::open(path.as_ref(), FileOpenOptions::default())?;
        let decision = ModeManager::new(settings.clone()).choose_for_open(file.metadata(), intent);
        let title = file
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Untitled")
            .to_string();

        match decision.mode {
            EditorMode::ViewAnalysis => Ok(Self {
                id,
                title,
                mode: EditorMode::ViewAnalysis,
                internal_engine: decision.internal_engine,
                mode_reason: decision.reason,
                user_notice: decision.user_notice,
                backing: DocumentBacking::View {
                    file,
                    viewport: ViewportEngine::default(),
                },
            }),
            EditorMode::Edit => {
                let bytes = file
                    .read_entire_if_under(settings.max_edit_load_bytes)
                    .with_context(|| format!("load {}", file.path().display()))?
                    .with_context(|| {
                        format!("{} exceeds max edit load threshold", file.path().display())
                    })?;
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let metadata = file.metadata().clone();
                Ok(Self {
                    id,
                    title,
                    mode: EditorMode::Edit,
                    internal_engine: decision.internal_engine,
                    mode_reason: decision.reason,
                    user_notice: decision.user_notice,
                    backing: DocumentBacking::Edit {
                        buffer: EditBuffer::from_text(&text),
                        path: Some(file.path().to_path_buf()),
                        metadata: Some(metadata),
                    },
                })
            }
        }
    }

    pub fn id(&self) -> DocumentId {
        self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn mode(&self) -> EditorMode {
        self.mode
    }

    pub fn mode_reason(&self) -> &str {
        &self.mode_reason
    }

    pub fn internal_engine_profile(&self) -> InternalEngineProfile {
        self.internal_engine
    }

    pub fn user_notice(&self) -> Option<&str> {
        self.user_notice.as_deref()
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.backing {
            DocumentBacking::View { file, .. } => Some(file.path()),
            DocumentBacking::Edit { path, .. } => path.as_deref(),
        }
    }

    pub fn metadata(&self) -> Option<&FileMetadata> {
        match &self.backing {
            DocumentBacking::View { file, .. } => Some(file.metadata()),
            DocumentBacking::Edit { metadata, .. } => metadata.as_ref(),
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.backing {
            DocumentBacking::View { .. } => false,
            DocumentBacking::Edit { buffer, .. } => buffer.is_dirty(),
        }
    }

    pub fn initial_viewport(&mut self, settings: &AppSettings) -> Result<Viewport> {
        self.viewport(ViewportRequest {
            anchor: ViewAnchor::Start,
            max_lines: settings.initial_viewport_lines,
            max_bytes: settings.initial_viewport_bytes,
        })
    }

    pub fn viewport(&mut self, request: ViewportRequest) -> Result<Viewport> {
        match &mut self.backing {
            DocumentBacking::View { file, viewport } => viewport.render(file, request),
            DocumentBacking::Edit { buffer, .. } => {
                let text = buffer.text();
                let lines = text
                    .lines()
                    .take(request.max_lines)
                    .enumerate()
                    .map(|(idx, line)| fastpad_line_index::LineSlice {
                        line_number: Some(idx as u64),
                        start: fastpad_file::ByteOffset(0),
                        end: fastpad_file::ByteOffset(0),
                        text: line.to_string(),
                        truncated: false,
                    })
                    .collect();
                Ok(Viewport {
                    anchor: request.anchor,
                    start: fastpad_file::ByteOffset(0),
                    end: fastpad_file::ByteOffset(text.len() as u64),
                    file_len: text.len() as u64,
                    lines,
                })
            }
        }
    }

    pub fn edit_buffer_mut(&mut self) -> Result<&mut EditBuffer> {
        match &mut self.backing {
            DocumentBacking::Edit { buffer, .. } => Ok(buffer),
            DocumentBacking::View { .. } => bail!("document is in View/Analysis Mode"),
        }
    }

    pub fn set_edit_text(&mut self, text: &str) -> Result<()> {
        if self.full_text_for_editing()? == text {
            return Ok(());
        }
        let buffer = self.edit_buffer_mut()?;
        let len = buffer.len_chars();
        buffer.replace(0..len, text)
    }

    pub fn full_text_for_editing(&self) -> Result<String> {
        match &self.backing {
            DocumentBacking::Edit { buffer, .. } => Ok(buffer.text()),
            DocumentBacking::View { .. } => bail!("document is in View/Analysis Mode"),
        }
    }

    pub fn has_save_path(&self) -> bool {
        matches!(
            &self.backing,
            DocumentBacking::Edit { path: Some(_), .. } | DocumentBacking::View { .. }
        )
    }

    pub fn search(&self, query: &SearchQuery, cancel: &CancellationToken) -> Result<SearchSummary> {
        match &self.backing {
            DocumentBacking::View { file, .. } => SearchEngine::search(file, query, cancel),
            DocumentBacking::Edit { buffer, .. } => {
                let text = buffer.text();
                SearchEngine::search_bytes(text.as_bytes(), query, cancel)
            }
        }
    }

    pub fn save(&mut self) -> Result<()> {
        match &mut self.backing {
            DocumentBacking::View { .. } => bail!("cannot save a read-only analysis document"),
            DocumentBacking::Edit { buffer, path, .. } => {
                let Some(path) = path else {
                    bail!("save-as path required for untitled document");
                };
                atomic_write(path, buffer.text().as_bytes())?;
                buffer.mark_clean();
                Ok(())
            }
        }
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<()> {
        match &mut self.backing {
            DocumentBacking::View { .. } => bail!("cannot save a read-only analysis document"),
            DocumentBacking::Edit {
                buffer,
                path: doc_path,
                metadata,
            } => {
                let path = path.as_ref().to_path_buf();
                atomic_write(&path, buffer.text().as_bytes())?;
                let file = FileHandle::open(&path, FileOpenOptions::default())?;
                *doc_path = Some(path.clone());
                *metadata = Some(file.metadata().clone());
                self.title = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Untitled")
                    .to_string();
                buffer.mark_clean();
                Ok(())
            }
        }
    }

    pub fn save_copy_as(&self, path: impl AsRef<Path>) -> Result<()> {
        match &self.backing {
            DocumentBacking::View { .. } => {
                bail!("cannot save copy from read-only analysis document")
            }
            DocumentBacking::Edit { buffer, .. } => atomic_write(path, buffer.text().as_bytes()),
        }
    }

    pub fn status_line(&self) -> String {
        let dirty = if self.is_dirty() { " modified" } else { "" };
        let size = self
            .metadata()
            .map(|metadata| format!(" {} bytes", metadata.len))
            .unwrap_or_default();
        let notice = self
            .user_notice
            .as_deref()
            .map(|notice| format!(" - {notice}"))
            .unwrap_or_default();
        format!(
            "{}{} - {}{}{}",
            self.title,
            dirty,
            self.mode.label(),
            notice,
            size
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DocumentId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WindowId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TabId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ViewId(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentViewState {
    pub anchor: ViewAnchor,
    pub next_anchor: Option<ViewAnchor>,
    pub cursor_offset: fastpad_file::ByteOffset,
    pub scroll_offset: fastpad_file::ByteOffset,
    pub zoom_level: f32,
    pub fold_state: Vec<fastpad_file::ByteRange>,
    pub bookmarks: Vec<fastpad_file::ByteOffset>,
    pub search_history: Vec<String>,
    pub active_filters: Vec<String>,
}

impl Default for DocumentViewState {
    fn default() -> Self {
        Self {
            anchor: ViewAnchor::Start,
            next_anchor: None,
            cursor_offset: fastpad_file::ByteOffset::ZERO,
            scroll_offset: fastpad_file::ByteOffset::ZERO,
            zoom_level: 1.0,
            fold_state: Vec::new(),
            bookmarks: Vec::new(),
            search_history: Vec::new(),
            active_filters: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tab {
    id: TabId,
    document_id: DocumentId,
    view_id: ViewId,
    view: DocumentViewState,
    pinned: bool,
    preview: bool,
    external_modified: bool,
}

impl Tab {
    pub fn id(&self) -> TabId {
        self.id
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn view_id(&self) -> ViewId {
        self.view_id
    }

    pub fn view(&self) -> &DocumentViewState {
        &self.view
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    pub fn is_preview(&self) -> bool {
        self.preview
    }

    pub fn is_external_modified(&self) -> bool {
        self.external_modified
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    id: WindowId,
    tabs: Vec<TabId>,
    active_tab: Option<TabId>,
}

impl WindowState {
    pub fn id(&self) -> WindowId {
        self.id
    }

    pub fn tabs(&self) -> &[TabId] {
        &self.tabs
    }

    pub fn active_tab(&self) -> Option<TabId> {
        self.active_tab
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TabSummary {
    pub id: TabId,
    pub view_id: ViewId,
    pub document_id: DocumentId,
    pub title: String,
    pub dirty: bool,
    pub read_only: bool,
    pub view_analysis: bool,
    pub external_modified: bool,
    pub pinned: bool,
    pub preview: bool,
    pub active: bool,
}

pub struct PendingOpenDocument {
    id: DocumentId,
    path: PathBuf,
    settings: AppSettings,
    intent: OpenIntent,
}

impl PendingOpenDocument {
    pub fn document_id(&self) -> DocumentId {
        self.id
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn open(self) -> Result<Document> {
        Document::open(self.id, self.path, &self.settings, self.intent)
    }
}

pub enum OpenTabRequest {
    Existing(TabId),
    Pending(PendingOpenDocument),
}

pub struct DocumentManager {
    settings: AppSettings,
    next_document_id: u64,
    next_tab_id: u64,
    next_view_id: u64,
    documents: BTreeMap<DocumentId, Arc<RwLock<Document>>>,
    tabs: BTreeMap<TabId, Tab>,
    path_index: BTreeMap<PathBuf, DocumentId>,
    windows: BTreeMap<WindowId, WindowState>,
    active_window: WindowId,
}

impl DocumentManager {
    pub fn new(settings: AppSettings) -> Self {
        let active_window = WindowId(1);
        let mut windows = BTreeMap::new();
        windows.insert(
            active_window,
            WindowState {
                id: active_window,
                tabs: Vec::new(),
                active_tab: None,
            },
        );
        Self {
            settings,
            next_document_id: 1,
            next_tab_id: 1,
            next_view_id: 1,
            documents: BTreeMap::new(),
            tabs: BTreeMap::new(),
            path_index: BTreeMap::new(),
            windows,
            active_window,
        }
    }

    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }

    pub fn open(&mut self, path: impl AsRef<Path>, intent: OpenIntent) -> Result<DocumentId> {
        let tab_id = self.open_tab(path, intent)?;
        Ok(self
            .tabs
            .get(&tab_id)
            .expect("newly opened tab exists")
            .document_id)
    }

    pub fn open_tab(&mut self, path: impl AsRef<Path>, intent: OpenIntent) -> Result<TabId> {
        let document_id = self.document_id_for_path(path.as_ref(), intent)?;
        Ok(self.create_tab_for_document(document_id))
    }

    pub fn begin_open_tab(
        &mut self,
        path: impl AsRef<Path>,
        intent: OpenIntent,
    ) -> Result<OpenTabRequest> {
        let path = path.as_ref();
        let index_path = normalized_path(path);
        if let Some(id) = self.path_index.get(&index_path).copied() {
            if self.documents.contains_key(&id) {
                return Ok(OpenTabRequest::Existing(self.create_tab_for_document(id)));
            }
        }

        let id = self.allocate_document_id();
        Ok(OpenTabRequest::Pending(PendingOpenDocument {
            id,
            path: path.to_path_buf(),
            settings: self.settings.clone(),
            intent,
        }))
    }

    pub fn finish_open_tab(&mut self, document: Document) -> TabId {
        let document_id = document.id();
        if let Some(path) = document.path().map(Path::to_path_buf) {
            let index_path = normalized_path(&path);
            if let Some(existing_id) = self.path_index.get(&index_path).copied() {
                if self.documents.contains_key(&existing_id) {
                    return self.create_tab_for_document(existing_id);
                }
            }
            self.path_index.insert(index_path, document_id);
        }
        self.documents
            .insert(document_id, Arc::new(RwLock::new(document)));
        self.create_tab_for_document(document_id)
    }

    pub fn new_untitled(&mut self) -> DocumentId {
        let tab_id = self.new_untitled_tab();
        self.tabs
            .get(&tab_id)
            .expect("newly created tab exists")
            .document_id
    }

    pub fn new_untitled_tab(&mut self) -> TabId {
        let id = self.allocate_document_id();
        let doc = Document::untitled(id);
        self.documents.insert(id, Arc::new(RwLock::new(doc)));
        self.create_tab_for_document(id)
    }

    pub fn duplicate_active_tab(&mut self) -> Option<TabId> {
        let document_id = self.active_tab().map(|tab| tab.document_id)?;
        Some(self.create_tab_for_document(document_id))
    }

    pub fn toggle_active_tab_pin(&mut self) -> Option<bool> {
        let tab_id = self.active_tab_id()?;
        let tab = self.tabs.get_mut(&tab_id)?;
        tab.pinned = !tab.pinned;
        Some(tab.pinned)
    }

    pub fn get(&self, id: DocumentId) -> Option<Arc<RwLock<Document>>> {
        self.documents.get(&id).cloned()
    }

    pub fn active(&self) -> Option<Arc<RwLock<Document>>> {
        self.active_tab().and_then(|tab| self.get(tab.document_id))
    }

    pub fn active_document_id(&self) -> Option<DocumentId> {
        self.active_tab().map(|tab| tab.document_id)
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.windows
            .get(&self.active_window)
            .and_then(|window| window.active_tab)
    }

    pub fn active_view_state(&self) -> Option<DocumentViewState> {
        self.active_tab().map(|tab| tab.view.clone())
    }

    pub fn update_active_view_state(
        &mut self,
        update: impl FnOnce(&mut DocumentViewState),
    ) -> bool {
        let Some(tab_id) = self.active_tab_id() else {
            return false;
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        update(&mut tab.view);
        true
    }

    pub fn set_active_tab(&mut self, tab_id: TabId) -> bool {
        if !self.tabs.contains_key(&tab_id) {
            return false;
        }
        let Some(window) = self.windows.get_mut(&self.active_window) else {
            return false;
        };
        if !window.tabs.contains(&tab_id) {
            return false;
        }
        window.active_tab = Some(tab_id);
        true
    }

    pub fn activate_next_tab(&mut self) -> bool {
        self.activate_relative_tab(1)
    }

    pub fn activate_previous_tab(&mut self) -> bool {
        self.activate_relative_tab(-1)
    }

    pub fn tab_summaries(&self) -> Vec<TabSummary> {
        let Some(window) = self.windows.get(&self.active_window) else {
            return Vec::new();
        };
        window
            .tabs
            .iter()
            .filter_map(|tab_id| self.tab_summary(*tab_id))
            .collect()
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn document_count(&self) -> usize {
        self.documents.len()
    }

    pub fn has_dirty_documents(&self) -> bool {
        self.documents
            .values()
            .any(|document| document.read().is_dirty())
    }

    fn document_id_for_path(&mut self, path: &Path, intent: OpenIntent) -> Result<DocumentId> {
        let index_path = normalized_path(path);
        if let Some(id) = self.path_index.get(&index_path).copied() {
            if self.documents.contains_key(&id) {
                return Ok(id);
            }
        }

        let id = self.allocate_document_id();
        let doc = Document::open(id, path, &self.settings, intent)?;
        if let Some(path) = doc.path() {
            self.path_index.insert(normalized_path(path), id);
        }
        self.documents.insert(id, Arc::new(RwLock::new(doc)));
        Ok(id)
    }

    fn create_tab_for_document(&mut self, document_id: DocumentId) -> TabId {
        let tab_id = self.allocate_tab_id();
        let view_id = self.allocate_view_id();
        self.tabs.insert(
            tab_id,
            Tab {
                id: tab_id,
                document_id,
                view_id,
                view: DocumentViewState::default(),
                pinned: false,
                preview: false,
                external_modified: false,
            },
        );
        let window = self
            .windows
            .get_mut(&self.active_window)
            .expect("active window exists");
        window.tabs.push(tab_id);
        window.active_tab = Some(tab_id);
        tab_id
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.active_tab_id().and_then(|id| self.tabs.get(&id))
    }

    fn activate_relative_tab(&mut self, delta: isize) -> bool {
        let Some(window) = self.windows.get_mut(&self.active_window) else {
            return false;
        };
        if window.tabs.is_empty() {
            return false;
        }
        let current = window.active_tab.unwrap_or(window.tabs[0]);
        let current_idx = window
            .tabs
            .iter()
            .position(|tab_id| *tab_id == current)
            .unwrap_or(0);
        let len = window.tabs.len() as isize;
        let next_idx = (current_idx as isize + delta).rem_euclid(len) as usize;
        window.active_tab = Some(window.tabs[next_idx]);
        true
    }

    fn tab_summary(&self, tab_id: TabId) -> Option<TabSummary> {
        let tab = self.tabs.get(&tab_id)?;
        let document = self.documents.get(&tab.document_id)?.read();
        let read_only = document
            .metadata()
            .map(|metadata| metadata.readonly)
            .unwrap_or(false);
        Some(TabSummary {
            id: tab.id,
            view_id: tab.view_id,
            document_id: tab.document_id,
            title: document.title().to_string(),
            dirty: document.is_dirty(),
            read_only,
            view_analysis: document.mode() == EditorMode::ViewAnalysis,
            external_modified: tab.external_modified,
            pinned: tab.pinned,
            preview: tab.preview,
            active: self.active_tab_id() == Some(tab.id),
        })
    }

    fn allocate_document_id(&mut self) -> DocumentId {
        let id = DocumentId(self.next_document_id);
        self.next_document_id += 1;
        id
    }

    fn allocate_tab_id(&mut self) -> TabId {
        let id = TabId(self.next_tab_id);
        self.next_tab_id += 1;
        id
    }

    fn allocate_view_id(&mut self) -> ViewId {
        let id = ViewId(self.next_view_id);
        self.next_view_id += 1;
        id
    }
}

fn normalized_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CommandId {
    Open,
    Save,
    Search,
    Copy,
    InsertText,
    DeleteSelection,
    Replace,
    ConvertToEditMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandCapability {
    Always,
    RequiresEdit,
    RequiresViewOrEdit,
}

#[derive(Debug, Clone)]
pub struct CommandRegistry {
    capabilities: BTreeMap<CommandId, CommandCapability>,
}

impl Default for CommandRegistry {
    fn default() -> Self {
        let mut capabilities = BTreeMap::new();
        capabilities.insert(CommandId::Open, CommandCapability::Always);
        capabilities.insert(CommandId::Save, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::Search, CommandCapability::RequiresViewOrEdit);
        capabilities.insert(CommandId::Copy, CommandCapability::RequiresViewOrEdit);
        capabilities.insert(CommandId::InsertText, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::DeleteSelection, CommandCapability::RequiresEdit);
        capabilities.insert(CommandId::Replace, CommandCapability::RequiresEdit);
        capabilities.insert(
            CommandId::ConvertToEditMode,
            CommandCapability::RequiresViewOrEdit,
        );
        Self { capabilities }
    }
}

impl CommandRegistry {
    pub fn is_enabled(&self, command: CommandId, mode: EditorMode) -> bool {
        match self.capabilities.get(&command).copied() {
            Some(CommandCapability::Always | CommandCapability::RequiresViewOrEdit) => true,
            Some(CommandCapability::RequiresEdit) => mode.is_editable(),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fastpad_file::{detect_file_kind, inspect_sample};
    use std::io::Write;
    use std::path::PathBuf;

    fn metadata_for(path: &str, len: u64, sample: &[u8]) -> FileMetadata {
        let intelligence = inspect_sample(sample);
        let path = PathBuf::from(path);
        let kind = detect_file_kind(&path, &intelligence);
        FileMetadata {
            path,
            len,
            readonly: false,
            modified: None,
            kind,
            intelligence,
        }
    }

    #[test]
    fn chooses_analysis_for_large_files() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        write!(tmp, "small").unwrap();
        let file = FileHandle::open(tmp.path(), FileOpenOptions::default()).unwrap();
        let settings = AppSettings {
            analysis_threshold_bytes: 1,
            ..Default::default()
        };
        let decision =
            ModeManager::new(settings).choose_for_open(file.metadata(), OpenIntent::default());

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
    }

    #[test]
    fn adaptive_selection_treats_small_file_with_extreme_line_as_analysis() {
        let sample = vec![b'x'; 40 * 1024];
        let metadata = metadata_for("single-line.txt", 20 * 1024 * 1024, &sample);
        let settings = AppSettings {
            huge_line_bytes: 32 * 1024,
            ..Default::default()
        };

        let decision = ModeManager::new(settings).choose_for_open_with_system(
            &metadata,
            OpenIntent::default(),
            SystemMemoryStatus::unconstrained_for_tests(),
        );

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
        assert_eq!(
            decision.internal_engine,
            InternalEngineProfile::HugeFileAnalysis
        );
        assert!(decision.reason.contains("long line"));
    }

    #[test]
    fn adaptive_selection_keeps_large_source_in_edit_mode_when_safe_to_load() {
        let metadata = metadata_for("main.rs", 300 * 1024 * 1024, b"fn main() {}\n");

        let decision = ModeManager::new(AppSettings::default()).choose_for_open_with_system(
            &metadata,
            OpenIntent::default(),
            SystemMemoryStatus::unconstrained_for_tests(),
        );

        assert_eq!(decision.mode, EditorMode::Edit);
        assert_eq!(
            decision.internal_engine,
            InternalEngineProfile::LargeOptimizedEdit
        );
        assert_eq!(
            decision.user_notice.as_deref(),
            Some("Large File Optimizations Enabled")
        );
    }

    #[test]
    fn adaptive_selection_routes_huge_csv_to_structured_analysis() {
        let metadata = metadata_for("events.csv", 5 * 1024 * 1024 * 1024, b"a,b\n1,2\n");

        let decision = ModeManager::new(AppSettings::default()).choose_for_open_with_system(
            &metadata,
            OpenIntent::default(),
            SystemMemoryStatus::unconstrained_for_tests(),
        );

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
        assert_eq!(
            decision.internal_engine,
            InternalEngineProfile::StructuredDataAnalysis
        );
    }

    #[test]
    fn adaptive_selection_uses_binary_inspection_for_binary_content() {
        let metadata = metadata_for("capture.bin", 8 * 1024, &[0, 1, 2, 3, 0, 0, 0, 0]);

        let decision = ModeManager::new(AppSettings::default()).choose_for_open_with_system(
            &metadata,
            OpenIntent::default(),
            SystemMemoryStatus::unconstrained_for_tests(),
        );

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
        assert_eq!(
            decision.internal_engine,
            InternalEngineProfile::BinaryInspection
        );
    }

    #[test]
    fn adaptive_selection_considers_memory_pressure() {
        let metadata = metadata_for("main.rs", 300 * 1024 * 1024, b"fn main() {}\n");
        let system = SystemMemoryStatus {
            available_bytes: Some(400 * 1024 * 1024),
            total_bytes: Some(8 * 1024 * 1024 * 1024),
            pressure: MemoryPressure::Critical,
        };

        let decision = ModeManager::new(AppSettings::default()).choose_for_open_with_system(
            &metadata,
            OpenIntent::default(),
            system,
        );

        assert_eq!(decision.mode, EditorMode::ViewAnalysis);
        assert!(decision.reason.contains("memory"));
    }

    #[test]
    fn force_edit_preserves_user_intent_and_marks_warning() {
        let metadata = metadata_for("server.log", 3 * 1024 * 1024 * 1024, b"line\n");

        let decision = ModeManager::new(AppSettings::default()).choose_for_open_with_system(
            &metadata,
            OpenIntent {
                force_analysis: false,
                force_edit: true,
            },
            SystemMemoryStatus::unconstrained_for_tests(),
        );

        assert_eq!(decision.mode, EditorMode::Edit);
        assert_eq!(
            decision.internal_engine,
            InternalEngineProfile::StreamingEdit
        );
        assert!(decision.requires_huge_edit_warning);
    }

    #[test]
    fn disables_edit_commands_in_analysis_mode() {
        let registry = CommandRegistry::default();
        assert!(!registry.is_enabled(CommandId::InsertText, EditorMode::ViewAnalysis));
        assert!(registry.is_enabled(CommandId::Search, EditorMode::ViewAnalysis));
    }

    #[test]
    fn edit_documents_expose_full_text_for_native_text_view() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        for idx in 0..150 {
            writeln!(tmp, "line {idx}").unwrap();
        }
        let settings = AppSettings::default();
        let doc =
            Document::open(DocumentId(1), tmp.path(), &settings, OpenIntent::default()).unwrap();

        assert_eq!(doc.mode(), EditorMode::Edit);
        assert_eq!(doc.full_text_for_editing().unwrap().lines().count(), 150);
    }

    #[test]
    fn save_as_updates_path_and_marks_clean() {
        let mut doc = Document::untitled(DocumentId(1));
        doc.set_edit_text("hello").unwrap();
        assert!(doc.is_dirty());

        let tmp = tempfile::NamedTempFile::new().unwrap();
        doc.save_as(tmp.path()).unwrap();

        assert!(!doc.is_dirty());
        assert!(doc.has_save_path());
        assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "hello");
    }

    #[test]
    fn opening_same_path_creates_lightweight_tabs_sharing_document() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "shared").unwrap();
        let mut manager = DocumentManager::new(AppSettings::default());

        let first_tab = manager.open_tab(tmp.path(), OpenIntent::default()).unwrap();
        let second_tab = manager.open_tab(tmp.path(), OpenIntent::default()).unwrap();

        assert_ne!(first_tab, second_tab);
        assert_eq!(manager.tab_count(), 2);
        assert_eq!(manager.document_count(), 1);

        let summaries = manager.tab_summaries();
        assert_eq!(summaries[0].document_id, summaries[1].document_id);
        assert_ne!(summaries[0].view_id, summaries[1].view_id);
        assert!(summaries[1].active);
    }

    #[test]
    fn background_open_finishes_as_active_tab() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "opened later").unwrap();
        let mut manager = DocumentManager::new(AppSettings::default());

        let pending = match manager
            .begin_open_tab(tmp.path(), OpenIntent::default())
            .unwrap()
        {
            OpenTabRequest::Pending(pending) => pending,
            OpenTabRequest::Existing(_) => panic!("new path should not already be open"),
        };

        assert_eq!(manager.document_count(), 0);
        assert_eq!(manager.tab_count(), 0);

        let document = pending.open().unwrap();
        let tab = manager.finish_open_tab(document);

        assert_eq!(manager.document_count(), 1);
        assert_eq!(manager.tab_count(), 1);
        assert_eq!(manager.active_tab_id(), Some(tab));
    }

    #[test]
    fn begin_open_tab_reuses_existing_document_immediately() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "shared").unwrap();
        let mut manager = DocumentManager::new(AppSettings::default());

        let first_tab = manager.open_tab(tmp.path(), OpenIntent::default()).unwrap();
        let second_tab = match manager
            .begin_open_tab(tmp.path(), OpenIntent::default())
            .unwrap()
        {
            OpenTabRequest::Existing(tab) => tab,
            OpenTabRequest::Pending(_) => panic!("existing path should not start background work"),
        };

        assert_ne!(first_tab, second_tab);
        assert_eq!(manager.document_count(), 1);
        assert_eq!(manager.tab_count(), 2);
        assert_eq!(manager.active_tab_id(), Some(second_tab));
    }

    #[test]
    fn duplicated_tab_shares_document_but_keeps_independent_view_state() {
        let mut tmp = tempfile::NamedTempFile::new().unwrap();
        writeln!(tmp, "line 1").unwrap();
        let mut manager = DocumentManager::new(AppSettings::default());
        let first_tab = manager.open_tab(tmp.path(), OpenIntent::default()).unwrap();
        manager.update_active_view_state(|view| {
            view.anchor = ViewAnchor::Byte(fastpad_file::ByteOffset(128));
        });

        let second_tab = manager.duplicate_active_tab().unwrap();
        manager.update_active_view_state(|view| {
            view.anchor = ViewAnchor::Byte(fastpad_file::ByteOffset(256));
        });

        let summaries = manager.tab_summaries();
        assert_eq!(manager.document_count(), 1);
        assert_eq!(summaries[0].document_id, summaries[1].document_id);
        assert_ne!(summaries[0].view_id, summaries[1].view_id);

        manager.set_active_tab(first_tab);
        assert_eq!(
            manager.active_view_state().unwrap().anchor,
            ViewAnchor::Byte(fastpad_file::ByteOffset(128))
        );
        manager.set_active_tab(second_tab);
        assert_eq!(
            manager.active_view_state().unwrap().anchor,
            ViewAnchor::Byte(fastpad_file::ByteOffset(256))
        );
    }

    #[test]
    fn next_previous_tab_switching_updates_active_document() {
        let mut first = tempfile::NamedTempFile::new().unwrap();
        let mut second = tempfile::NamedTempFile::new().unwrap();
        writeln!(first, "first").unwrap();
        writeln!(second, "second").unwrap();
        let mut manager = DocumentManager::new(AppSettings::default());

        let first_tab = manager
            .open_tab(first.path(), OpenIntent::default())
            .unwrap();
        let second_tab = manager
            .open_tab(second.path(), OpenIntent::default())
            .unwrap();

        assert_eq!(manager.active_tab_id(), Some(second_tab));
        assert!(manager.activate_previous_tab());
        assert_eq!(manager.active_tab_id(), Some(first_tab));
        assert!(manager.activate_next_tab());
        assert_eq!(manager.active_tab_id(), Some(second_tab));
    }
}
