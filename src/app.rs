use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::{
    config::Config,
    microcms::{
        ApiInfo, ContentCollection, ContentCollectionKind, ContentWriteStatus, PublicationStatus,
    },
};

pub const PAGE_LIMIT: usize = 20;
const SYSTEM_METADATA_FIELDS: [&str; 6] = [
    "id",
    "_id",
    "createdAt",
    "updatedAt",
    "publishedAt",
    "revisedAt",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    EndpointPicker,
    ContentBrowser,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputTarget {
    Search,
    Filters,
    Orders,
    CreateWithId(ContentWriteStatus),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentPublicationState {
    Published,
    Draft,
    Closed,
    PublishedAndDraft,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PendingConfirmation {
    Delete {
        content_ids: Vec<String>,
    },
    Create {
        value: Value,
        status: ContentWriteStatus,
    },
    PutCreate {
        content_id: String,
        value: Value,
        status: ContentWriteStatus,
    },
    Update {
        content_id: String,
        value: Value,
        status: ContentWriteStatus,
    },
    PublicationStatus {
        content_ids: Vec<String>,
        status: PublicationStatus,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    MissingConfig(Vec<String>),
    LoadingApis,
    ApisLoaded,
    LoadingContents,
    ContentsLoaded,
    Error(String),
}

#[derive(Debug)]
pub struct App {
    pub config: Config,
    pub apis: Vec<ApiInfo>,
    pub api_selected: usize,
    pub endpoint: Option<String>,
    pub items: Vec<Value>,
    pub content_selected: usize,
    pub offset: usize,
    pub limit: usize,
    pub total_count: Option<usize>,
    pub content_kind: ContentCollectionKind,
    pub content_statuses: HashMap<String, ContentPublicationState>,
    pub selected_content_ids: HashSet<String>,
    pub create_template: Option<Value>,
    pub content_field_order: Vec<String>,
    pub search_query: Option<String>,
    pub filters: Option<String>,
    pub orders: Option<String>,
    pub input_target: Option<InputTarget>,
    pub input_buffer: String,
    pub preview_fullscreen: bool,
    pub preview_scroll: u16,
    pub help_open: bool,
    pub screen: Screen,
    pub state: LoadState,
    pub pending_confirmation: Option<PendingConfirmation>,
    pub message: Option<String>,
    pub should_quit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    Back,
    MoveDown,
    MoveUp,
    ToggleSelect,
    Select,
    Reload,
    NextPage,
    PrevPage,
    Create,
    CreateDraft,
    CreateWithId,
    CreateWithIdDraft,
    Edit,
    EditDraft,
    DeleteRequest,
    ConfirmPending,
    CancelPending,
    EditSearch,
    EditFilters,
    EditOrders,
    ClearQuery,
    Publish,
    Draft,
    InputChar(char),
    InputBackspace,
    InputApply,
    InputCancel,
    TogglePreviewFullscreen,
    ClosePreviewFullscreen,
    PreviewScrollDown,
    PreviewScrollUp,
    PreviewScrollTop,
    PreviewScrollBottom,
    PreviewNextContent,
    PreviewPrevContent,
    ToggleHelp,
    CloseHelp,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    None,
    FetchApis,
    FetchContents,
    Create {
        template: Value,
        status: ContentWriteStatus,
    },
    CreateWithId {
        content_id: String,
        template: Value,
        status: ContentWriteStatus,
    },
    Update {
        content_id: String,
        value: Value,
        status: ContentWriteStatus,
    },
    Confirmed(PendingConfirmation),
}

#[derive(Debug)]
pub enum AppEvent {
    ApisLoaded(Vec<ApiInfo>),
    ContentsLoaded {
        endpoint: String,
        collection: ContentCollection,
        statuses: HashMap<String, ContentPublicationState>,
        status_warning: Option<String>,
        create_template: Option<Value>,
        content_field_order: Option<Vec<String>>,
        schema_warning: Option<String>,
    },
    FetchFailed {
        endpoint: Option<String>,
        error: String,
    },
    MutationSucceeded {
        endpoint: String,
        message: String,
    },
    StatusSucceeded {
        endpoint: String,
        message: String,
    },
    MutationFailed {
        endpoint: String,
        error: String,
    },
}

impl App {
    pub fn new(config: Config) -> Self {
        let mut missing = Vec::new();
        if is_missing(&config.service_id) {
            missing.push("service ID (--service-id or MICROCMS_SERVICE_ID)".to_string());
        }
        if is_missing(&config.api_key) {
            missing.push("API key (--api-key or MICROCMS_API_KEY)".to_string());
        }

        let state = if missing.is_empty() {
            LoadState::LoadingApis
        } else {
            LoadState::MissingConfig(missing)
        };
        let endpoint = config.endpoint.clone();
        Self {
            config,
            apis: Vec::new(),
            api_selected: 0,
            endpoint,
            items: Vec::new(),
            content_selected: 0,
            offset: 0,
            limit: PAGE_LIMIT,
            total_count: None,
            content_kind: ContentCollectionKind::List,
            content_statuses: HashMap::new(),
            selected_content_ids: HashSet::new(),
            create_template: None,
            content_field_order: Vec::new(),
            search_query: None,
            filters: None,
            orders: None,
            input_target: None,
            input_buffer: String::new(),
            preview_fullscreen: false,
            preview_scroll: 0,
            help_open: false,
            screen: Screen::EndpointPicker,
            state,
            pending_confirmation: None,
            message: None,
            should_quit: false,
        }
    }

    pub fn apply_action(&mut self, action: Action) -> Command {
        match action {
            Action::Quit => self.should_quit = true,
            Action::ToggleHelp => self.help_open = !self.help_open,
            Action::CloseHelp => self.help_open = false,
            Action::Back => {
                if self.screen == Screen::ContentBrowser {
                    self.help_open = false;
                    self.close_preview();
                    self.screen = Screen::EndpointPicker;
                    self.state = LoadState::ApisLoaded;
                    self.pending_confirmation = None;
                    self.selected_content_ids.clear();
                    self.message = None;
                }
            }
            Action::MoveDown => match self.screen {
                Screen::EndpointPicker => {
                    if self.api_selected + 1 < self.apis.len() {
                        self.api_selected += 1;
                    }
                }
                Screen::ContentBrowser => {
                    if self.content_selected + 1 < self.items.len() {
                        self.content_selected += 1;
                    }
                }
            },
            Action::MoveUp => match self.screen {
                Screen::EndpointPicker => {
                    self.api_selected = self.api_selected.saturating_sub(1);
                }
                Screen::ContentBrowser => {
                    self.content_selected = self.content_selected.saturating_sub(1);
                }
            },
            Action::ToggleSelect => {
                if self.is_object_api() {
                    self.message = Some("Object API does not support bulk selection.".into());
                    return Command::None;
                }
                if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
                    if let Some(content_id) = self.selected_content_id() {
                        if !self.selected_content_ids.remove(&content_id) {
                            self.selected_content_ids.insert(content_id);
                        }
                        self.message = None;
                    } else {
                        self.message =
                            Some("Selected content has no id or _id; cannot select.".into());
                    }
                }
            }
            Action::Select => {
                if self.screen == Screen::EndpointPicker
                    && !matches!(self.state, LoadState::LoadingApis)
                {
                    if let Some(api) = self.apis.get(self.api_selected) {
                        self.help_open = false;
                        let endpoint = api.endpoint.clone();
                        self.close_preview();
                        self.endpoint = Some(endpoint);
                        self.screen = Screen::ContentBrowser;
                        self.offset = 0;
                        self.limit = PAGE_LIMIT;
                        self.items.clear();
                        self.content_selected = 0;
                        self.total_count = None;
                        self.content_kind = ContentCollectionKind::List;
                        self.create_template = None;
                        self.content_field_order.clear();
                        self.selected_content_ids.clear();
                        self.pending_confirmation = None;
                        self.state = LoadState::LoadingContents;
                        self.message = None;
                        return Command::FetchContents;
                    }
                }
            }
            Action::Reload => match self.screen {
                Screen::EndpointPicker
                    if !matches!(
                        self.state,
                        LoadState::MissingConfig(_) | LoadState::LoadingApis
                    ) =>
                {
                    self.help_open = false;
                    self.state = LoadState::LoadingApis;
                    self.message = None;
                    return Command::FetchApis;
                }
                Screen::ContentBrowser
                    if self.endpoint.is_some()
                        && !matches!(self.state, LoadState::LoadingContents) =>
                {
                    self.help_open = false;
                    self.close_preview();
                    self.selected_content_ids.clear();
                    self.pending_confirmation = None;
                    self.state = LoadState::LoadingContents;
                    self.message = None;
                    return Command::FetchContents;
                }
                _ => {}
            },
            Action::NextPage => {
                if self.is_object_api() {
                    self.message = Some("Object API has no pages.".into());
                    return Command::None;
                }
                let has_next = self
                    .total_count
                    .map_or(true, |total| self.offset.saturating_add(self.limit) < total);
                if self.can_page() && has_next {
                    self.help_open = false;
                    self.close_preview();
                    self.offset = self.offset.saturating_add(self.limit);
                    self.selected_content_ids.clear();
                    self.pending_confirmation = None;
                    self.state = LoadState::LoadingContents;
                    self.message = None;
                    return Command::FetchContents;
                }
            }
            Action::PrevPage => {
                if self.is_object_api() {
                    self.message = Some("Object API has no pages.".into());
                    return Command::None;
                }
                if self.can_page() && self.offset > 0 {
                    self.help_open = false;
                    self.close_preview();
                    self.offset = self.offset.saturating_sub(self.limit);
                    self.selected_content_ids.clear();
                    self.pending_confirmation = None;
                    self.state = LoadState::LoadingContents;
                    self.message = None;
                    return Command::FetchContents;
                }
            }
            Action::TogglePreviewFullscreen => {
                if self.screen == Screen::ContentBrowser {
                    if self.items.is_empty() {
                        self.message = Some("No content selected.".into());
                    } else {
                        self.preview_fullscreen = !self.preview_fullscreen;
                        self.preview_scroll = 0;
                        if self.preview_fullscreen {
                            self.selected_content_ids.clear();
                        }
                        self.message = None;
                    }
                }
            }
            Action::ClosePreviewFullscreen => self.close_preview(),
            Action::PreviewScrollDown => {
                if self.preview_fullscreen {
                    self.preview_scroll = self.preview_scroll.saturating_add(1);
                }
            }
            Action::PreviewScrollUp => {
                if self.preview_fullscreen {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1);
                }
            }
            Action::PreviewScrollTop => {
                if self.preview_fullscreen {
                    self.preview_scroll = 0;
                }
            }
            Action::PreviewScrollBottom => {
                if self.preview_fullscreen {
                    self.preview_scroll = u16::MAX;
                }
            }
            Action::PreviewNextContent => {
                if self.preview_fullscreen && self.content_selected + 1 < self.items.len() {
                    self.content_selected += 1;
                    self.preview_scroll = 0;
                }
            }
            Action::PreviewPrevContent => {
                if self.preview_fullscreen {
                    self.content_selected = self.content_selected.saturating_sub(1);
                    self.preview_scroll = 0;
                }
            }
            Action::Create => {
                return self.create_command(ContentWriteStatus::Default);
            }
            Action::CreateDraft => {
                return self.create_command(ContentWriteStatus::Draft);
            }
            Action::CreateWithId => {
                if self.is_object_api() {
                    self.message = Some("Object API does not support Content API create.".into());
                    return Command::None;
                }
                if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
                    self.message = None;
                    self.begin_input(InputTarget::CreateWithId(ContentWriteStatus::Default));
                }
            }
            Action::CreateWithIdDraft => {
                if self.is_object_api() {
                    self.message = Some("Object API does not support Content API create.".into());
                    return Command::None;
                }
                if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
                    self.message = None;
                    self.begin_input(InputTarget::CreateWithId(ContentWriteStatus::Draft));
                }
            }
            Action::Edit => {
                return self.edit_command(ContentWriteStatus::Default);
            }
            Action::EditDraft => {
                return self.edit_command(ContentWriteStatus::Draft);
            }
            Action::DeleteRequest => {
                if self.is_object_api() {
                    self.message = Some("Object API does not support Content API delete.".into());
                    return Command::None;
                }
                if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
                    let content_ids = self.selected_or_current_content_ids();
                    if content_ids.is_empty() {
                        self.message =
                            Some("Selected content has no id or _id; cannot delete.".into());
                    } else {
                        let count = content_ids.len();
                        self.pending_confirmation =
                            Some(PendingConfirmation::Delete { content_ids });
                        self.message = Some(if count == 1 {
                            "Delete selected content? Press y to confirm or n/Esc to cancel.".into()
                        } else {
                            format!(
                                "Delete {count} selected contents? Press y to confirm or n/Esc to cancel."
                            )
                        });
                    }
                }
            }
            Action::ConfirmPending => {
                if let Some(confirmation) = self.pending_confirmation.take() {
                    self.close_preview();
                    self.message = Some("Operation confirmed; applying...".into());
                    return Command::Confirmed(confirmation);
                }
            }
            Action::CancelPending => {
                if self.pending_confirmation.take().is_some() {
                    self.message = Some("Operation cancelled.".into());
                }
            }
            Action::EditSearch if self.screen == Screen::ContentBrowser => {
                self.begin_input(InputTarget::Search);
            }
            Action::EditFilters if self.screen == Screen::ContentBrowser => {
                self.begin_input(InputTarget::Filters);
            }
            Action::EditOrders if self.screen == Screen::ContentBrowser => {
                self.begin_input(InputTarget::Orders);
            }
            Action::ClearQuery if self.screen == Screen::ContentBrowser => {
                self.help_open = false;
                self.close_preview();
                self.search_query = None;
                self.filters = None;
                self.orders = None;
                self.offset = 0;
                self.selected_content_ids.clear();
                self.pending_confirmation = None;
                self.state = LoadState::LoadingContents;
                self.message = Some("Query cleared.".into());
                return Command::FetchContents;
            }
            Action::Publish if self.screen == Screen::ContentBrowser => {
                return self.publication_status_command(PublicationStatus::Publish);
            }
            Action::Draft if self.screen == Screen::ContentBrowser => {
                return self.publication_status_command(PublicationStatus::Draft);
            }
            Action::InputChar(character) => {
                if self.input_target.is_some() {
                    self.input_buffer.push(character);
                }
            }
            Action::InputBackspace => {
                if self.input_target.is_some() {
                    self.input_buffer.pop();
                }
            }
            Action::InputApply => {
                if let Some(target) = self.input_target.take() {
                    let input = self.input_buffer.trim().to_string();
                    self.input_buffer.clear();
                    if let InputTarget::CreateWithId(status) = target {
                        if self.is_object_api() {
                            self.message =
                                Some("Object API does not support Content API create.".into());
                            return Command::None;
                        }
                        if input.is_empty() {
                            self.message = Some("Content ID is required.".into());
                            return Command::None;
                        }
                        if let Some(template) = &self.create_template {
                            self.message = None;
                            return Command::CreateWithId {
                                content_id: input,
                                template: template.clone(),
                                status,
                            };
                        }
                        self.message = Some("Schema unavailable; cannot create content.".into());
                        return Command::None;
                    }
                    let value = (!input.is_empty()).then_some(input);
                    match target {
                        InputTarget::Search => self.search_query = value,
                        InputTarget::Filters => self.filters = value,
                        InputTarget::Orders => self.orders = value,
                        InputTarget::CreateWithId(_) => unreachable!(),
                    }
                    self.offset = 0;
                    self.help_open = false;
                    self.close_preview();
                    self.pending_confirmation = None;
                    self.selected_content_ids.clear();
                    self.message = None;
                    self.state = LoadState::LoadingContents;
                    return Command::FetchContents;
                }
            }
            Action::InputCancel => {
                self.input_target = None;
                self.input_buffer.clear();
            }
            Action::EditSearch
            | Action::EditFilters
            | Action::EditOrders
            | Action::ClearQuery
            | Action::Publish
            | Action::Draft => {}
        }
        Command::None
    }

    pub fn apply_event(&mut self, event: AppEvent) -> Command {
        match event {
            AppEvent::ApisLoaded(apis) => {
                self.help_open = false;
                self.apis = apis;
                self.api_selected = self
                    .endpoint
                    .as_deref()
                    .and_then(|endpoint| self.apis.iter().position(|api| api.endpoint == endpoint))
                    .unwrap_or(0);
                self.screen = Screen::EndpointPicker;
                self.state = LoadState::ApisLoaded;
            }
            AppEvent::ContentsLoaded {
                endpoint,
                collection,
                statuses,
                status_warning,
                create_template,
                content_field_order,
                schema_warning,
            } => {
                if !self.accepts_contents_event(&endpoint) {
                    return Command::None;
                }
                self.help_open = false;
                self.close_preview();
                self.input_target = None;
                self.input_buffer.clear();
                let is_object = collection.kind == ContentCollectionKind::Object;
                self.content_kind = collection.kind;
                self.offset = collection.offset;
                self.limit = collection.limit;
                self.total_count = Some(collection.total_count);
                self.items = collection.contents;
                self.content_statuses = if is_object { HashMap::new() } else { statuses };
                self.selected_content_ids.clear();
                self.pending_confirmation = None;
                if let Some(template) = create_template {
                    self.create_template = Some(template);
                }
                if let Some(field_order) = content_field_order {
                    self.content_field_order = field_order;
                }
                self.content_selected = 0;
                self.screen = Screen::ContentBrowser;
                self.state = LoadState::ContentsLoaded;
                if let Some(warning) = (!is_object).then_some(status_warning).flatten() {
                    self.message = Some(match self.message.take() {
                        Some(message) => format!("{message} {warning}"),
                        None => warning,
                    });
                }
                if let Some(warning) = schema_warning {
                    self.message = Some(match self.message.take() {
                        Some(message) => format!("{message} {warning}"),
                        None => warning,
                    });
                }
            }
            AppEvent::FetchFailed { endpoint, error } => {
                let should_apply = match endpoint.as_deref() {
                    Some(endpoint) => self.accepts_contents_event(endpoint),
                    None => {
                        self.screen == Screen::EndpointPicker
                            && matches!(self.state, LoadState::LoadingApis)
                    }
                };
                if !should_apply {
                    return Command::None;
                }
                self.state = LoadState::Error(error);
                self.message = None;
            }
            AppEvent::MutationSucceeded { endpoint, message } => {
                if !self.accepts_mutation_event(&endpoint) {
                    return Command::None;
                }
                self.help_open = false;
                self.close_preview();
                self.pending_confirmation = None;
                self.selected_content_ids.clear();
                self.message = Some(message);
                self.state = LoadState::LoadingContents;
                return Command::FetchContents;
            }
            AppEvent::StatusSucceeded { endpoint, message } => {
                if !self.accepts_mutation_event(&endpoint) {
                    return Command::None;
                }
                self.help_open = false;
                self.close_preview();
                self.pending_confirmation = None;
                self.selected_content_ids.clear();
                self.message = Some(format!(
                    "{message} Item may be hidden by permissions or current query/filter."
                ));
                self.state = LoadState::LoadingContents;
                return Command::FetchContents;
            }
            AppEvent::MutationFailed { endpoint, error } => {
                if !self.accepts_mutation_event(&endpoint) {
                    return Command::None;
                }
                self.pending_confirmation = None;
                self.message = Some(format!("error: {error}"));
            }
        }
        Command::None
    }

    fn can_page(&self) -> bool {
        self.screen == Screen::ContentBrowser
            && self.endpoint.is_some()
            && !matches!(self.state, LoadState::LoadingContents)
    }

    fn create_command(&mut self, status: ContentWriteStatus) -> Command {
        if self.is_object_api() {
            self.message = Some("Object API does not support Content API create.".into());
            return Command::None;
        }
        if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
            if let Some(template) = &self.create_template {
                self.message = None;
                return Command::Create {
                    template: template.clone(),
                    status,
                };
            }
            self.message = Some("Schema unavailable; cannot create content.".into());
        }
        Command::None
    }

    fn edit_command(&mut self, status: ContentWriteStatus) -> Command {
        if self.is_object_api() {
            self.message =
                Some("Object API edit is not supported by the documented Content API.".into());
            return Command::None;
        }
        if self.screen == Screen::ContentBrowser && self.pending_confirmation.is_none() {
            if let Some(value) = self.items.get(self.content_selected) {
                if let Some(content_id) = content_id(value) {
                    return Command::Update {
                        content_id: content_id.to_string(),
                        value: editable_payload(self.create_template.as_ref(), value),
                        status,
                    };
                }
            }
            self.message = Some("Selected content has no id or _id; cannot edit.".into());
        }
        Command::None
    }

    fn close_preview(&mut self) {
        self.preview_fullscreen = false;
        self.preview_scroll = 0;
    }

    fn selected_content_id(&self) -> Option<String> {
        self.items
            .get(self.content_selected)
            .and_then(content_id)
            .map(str::to_string)
    }

    fn selected_or_current_content_ids(&self) -> Vec<String> {
        if self.selected_content_ids.is_empty() {
            return self.selected_content_id().into_iter().collect();
        }

        self.items
            .iter()
            .filter_map(content_id)
            .filter(|content_id| self.selected_content_ids.contains(*content_id))
            .map(str::to_string)
            .collect()
    }

    fn publication_status_command(&mut self, status: PublicationStatus) -> Command {
        if self.is_object_api() {
            self.message = Some("Object API publication status changes are not supported.".into());
            return Command::None;
        }
        let content_ids = self.selected_or_current_content_ids();
        if !content_ids.is_empty() {
            self.pending_confirmation = Some(PendingConfirmation::PublicationStatus {
                content_ids,
                status,
            });
            self.message = Some("Confirm publication status change.".into());
            return Command::None;
        }
        self.message =
            Some("Selected content has no id or _id; cannot change publication status.".into());
        Command::None
    }

    pub fn request_confirmation(&mut self, confirmation: PendingConfirmation) {
        self.pending_confirmation = Some(confirmation);
        self.message = Some("Confirm operation.".into());
    }

    fn is_object_api(&self) -> bool {
        self.screen == Screen::ContentBrowser && self.content_kind == ContentCollectionKind::Object
    }

    fn accepts_contents_event(&self, endpoint: &str) -> bool {
        self.screen == Screen::ContentBrowser
            && matches!(self.state, LoadState::LoadingContents)
            && self.endpoint.as_deref() == Some(endpoint)
    }

    fn accepts_mutation_event(&self, endpoint: &str) -> bool {
        self.screen == Screen::ContentBrowser && self.endpoint.as_deref() == Some(endpoint)
    }

    fn begin_input(&mut self, target: InputTarget) {
        self.input_buffer = match target {
            InputTarget::Search => self.search_query.clone(),
            InputTarget::Filters => self.filters.clone(),
            InputTarget::Orders => self.orders.clone(),
            InputTarget::CreateWithId(_) => None,
        }
        .unwrap_or_default();
        self.input_target = Some(target);
    }

    pub fn publication_state_for(&self, value: &Value) -> ContentPublicationState {
        content_id(value)
            .and_then(|id| self.content_statuses.get(id))
            .copied()
            .unwrap_or(ContentPublicationState::Unknown)
    }
}

pub fn content_id(value: &Value) -> Option<&str> {
    let object = value.as_object()?;
    object
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.trim().is_empty())
        .or_else(|| object.get("_id").and_then(Value::as_str))
        .filter(|id| !id.trim().is_empty())
}

pub fn content_publication_state(statuses: &[String]) -> ContentPublicationState {
    let has_status = |expected: &str| {
        statuses
            .iter()
            .any(|status| status.eq_ignore_ascii_case(expected))
    };
    if has_status("PUBLISH_AND_DRAFT") || (has_status("PUBLISH") && has_status("DRAFT")) {
        ContentPublicationState::PublishedAndDraft
    } else if has_status("CLOSED") {
        ContentPublicationState::Closed
    } else if has_status("PUBLISH") {
        ContentPublicationState::Published
    } else if has_status("DRAFT") {
        ContentPublicationState::Draft
    } else {
        ContentPublicationState::Unknown
    }
}

pub fn sanitized_payload(value: &Value) -> Value {
    let mut payload = value.clone();
    if let Some(object) = payload.as_object_mut() {
        for field in SYSTEM_METADATA_FIELDS {
            object.remove(field);
        }
    }
    payload
}

pub fn editable_payload(template: Option<&Value>, content: &Value) -> Value {
    let sanitized = sanitized_payload(content);
    let (Some(template), Some(content)) =
        (template.and_then(Value::as_object), sanitized.as_object())
    else {
        return sanitized;
    };

    let mut editable = template.clone();
    editable.extend(content.clone());
    Value::Object(editable)
}

pub fn ordered_content_for_display(
    value: &Value,
    field_order: &[String],
    include_system: bool,
) -> Value {
    let Some(object) = value.as_object() else {
        return value.clone();
    };
    let mut ordered = serde_json::Map::new();
    if include_system {
        for field in SYSTEM_METADATA_FIELDS {
            if let Some(value) = object.get(field) {
                ordered.insert(field.to_string(), value.clone());
            }
        }
    }
    for field in field_order {
        if !is_system_metadata_field(field) {
            if let Some(value) = object.get(field) {
                ordered.insert(field.clone(), value.clone());
            }
        }
    }
    for (field, value) in object {
        if (!is_system_metadata_field(field) || include_system) && !ordered.contains_key(field) {
            ordered.insert(field.clone(), value.clone());
        }
    }
    Value::Object(ordered)
}

pub fn create_template_from_api_schema(value: &Value) -> Option<Value> {
    let (fields, official_api_fields) = find_field_array(value)?;
    let mut template = serde_json::Map::new();
    for field in fields {
        let Some(field) = field.as_object() else {
            continue;
        };
        let field_id = if official_api_fields {
            string_field(field, &["fieldId"])
        } else {
            schema_field_id(field)
        };
        let Some(field_id) = field_id else {
            continue;
        };
        if field_id.is_empty() || is_system_field(field_id) {
            continue;
        }
        let kind =
            string_field(field, &["kind", "type", "fieldType", "dataType"]).unwrap_or_default();
        let is_multiple = ["multiple", "multipleSelect", "isMultiple"]
            .iter()
            .find_map(|key| field.get(*key).and_then(Value::as_bool));
        template.insert(
            field_id.to_string(),
            initial_value_for_kind(kind, is_multiple),
        );
    }
    (!template.is_empty()).then_some(Value::Object(template))
}

pub fn content_field_order_from_api_schema(value: &Value) -> Vec<String> {
    let Some((fields, official_api_fields)) = find_field_array(value) else {
        return Vec::new();
    };
    fields
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|field| {
            if official_api_fields {
                string_field(field, &["fieldId"])
            } else {
                schema_field_id(field)
            }
        })
        .filter(|field_id| !field_id.is_empty() && !is_system_field(field_id))
        .map(str::to_string)
        .collect()
}

fn find_field_array(value: &Value) -> Option<(&Vec<Value>, bool)> {
    match value {
        Value::Object(object) => {
            if let Some(fields) = object.get("apiFields").and_then(Value::as_array) {
                if !fields.is_empty() {
                    return Some((fields, true));
                }
            }
            if let Some(fields) = object.get("fields").and_then(Value::as_array) {
                if !fields.is_empty() {
                    return Some((fields, false));
                }
            }
            if let Some(schema) = object.get("schema") {
                if let Some(fields) = schema.as_array() {
                    if !fields.is_empty() {
                        return Some((fields, false));
                    }
                }
                if let Some(fields) = find_field_array(schema) {
                    return Some(fields);
                }
            }
            object
                .iter()
                .filter(|(key, _)| key.as_str() != "customFields")
                .find_map(|(_, value)| find_field_array(value))
        }
        Value::Array(values) => values.iter().find_map(find_field_array),
        _ => None,
    }
}

fn string_field<'a>(object: &'a serde_json::Map<String, Value>, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| object.get(*key).and_then(Value::as_str))
}

fn schema_field_id(object: &serde_json::Map<String, Value>) -> Option<&str> {
    string_field(object, &["fieldId", "id", "key"])
}

fn is_system_metadata_field(field_id: &str) -> bool {
    SYSTEM_METADATA_FIELDS
        .iter()
        .any(|system| field_id.eq_ignore_ascii_case(system))
}

fn is_system_field(field_id: &str) -> bool {
    const SYSTEM_FIELDS: [&str; 14] = [
        "id",
        "_id",
        "createdAt",
        "updatedAt",
        "publishedAt",
        "revisedAt",
        "draftKey",
        "status",
        "customStatus",
        "createdBy",
        "updatedBy",
        "reservationTime",
        "closedAt",
        "_status",
    ];
    SYSTEM_FIELDS
        .iter()
        .any(|system| field_id.eq_ignore_ascii_case(system))
}

fn initial_value_for_kind(kind: &str, is_multiple: Option<bool>) -> Value {
    let kind: String = kind
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if kind == "select" || kind == "selectfield" {
        return if is_multiple == Some(false) {
            Value::String(String::new())
        } else {
            Value::Array(Vec::new())
        };
    }

    if is_multiple == Some(true)
        || kind.contains("multiple")
        || kind.ends_with("list")
        || matches!(
            kind.as_str(),
            "repeat" | "repeater" | "array" | "images" | "files" | "references" | "selects"
        )
    {
        Value::Array(Vec::new())
    } else if matches!(
        kind.as_str(),
        "boolean" | "bool" | "number" | "integer" | "float" | "decimal"
    ) {
        Value::Null
    } else if matches!(
        kind.as_str(),
        "contentreference" | "reference" | "relation" | "contentrelation"
    ) {
        Value::Object(serde_json::Map::new())
    } else if matches!(
        kind.as_str(),
        "object"
            | "objectfield"
            | "custom"
            | "customfield"
            | "extended"
            | "extendedfield"
            | "extension"
            | "extensionfield"
    ) {
        Value::Object(serde_json::Map::new())
    } else if matches!(
        kind.as_str(),
        "text"
            | "string"
            | "textfield"
            | "textarea"
            | "multilinetext"
            | "markdown"
            | "image"
            | "file"
            | "media"
            | "date"
            | "datetime"
            | "richeditorv2"
            | "richeditor"
            | "oldricheditor"
    ) {
        Value::String(String::new())
    } else {
        Value::Null
    }
}

fn is_missing(value: &Option<String>) -> bool {
    value
        .as_deref()
        .map_or(true, |value| value.trim().is_empty())
}

pub fn content_label(value: &Value, field_order: &[String]) -> String {
    if let Some(object) = value.as_object() {
        for field in field_order {
            if let Some(label) = object.get(field).and_then(displayable_label) {
                return label;
            }
        }
        if let Some(id) = content_id(value) {
            return id.to_string();
        }
    }

    let compact = serde_json::to_string(value).unwrap_or_else(|_| "<invalid JSON>".to_string());
    truncate_chars(&compact, 80)
}

fn displayable_label(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) if value.is_empty() => None,
        Value::String(value) => Some(value.clone()),
        Value::Number(_) | Value::Bool(_) => Some(value.to_string()),
        Value::Array(values) if values.is_empty() => None,
        Value::Object(object) if object.is_empty() => None,
        Value::Array(_) | Value::Object(_) => serde_json::to_string(value).ok(),
    }
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }

    let content_chars = max_chars.saturating_sub(3);
    let truncated: String = value.chars().take(content_chars).collect();
    format!("{truncated}...")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn credentials_only_config() -> Config {
        Config {
            service_id: Some("service".into()),
            api_key: Some("key".into()),
            endpoint: None,
        }
    }

    fn contents_loaded_event(endpoint: &str, item_id: &str) -> AppEvent {
        AppEvent::ContentsLoaded {
            endpoint: endpoint.into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::List,
                total_count: 1,
                offset: 0,
                limit: PAGE_LIMIT,
                contents: vec![json!({"id": item_id})],
            },
            statuses: HashMap::new(),
            status_warning: None,
            create_template: None,
            content_field_order: None,
            schema_warning: None,
        }
    }

    #[test]
    fn app_does_not_require_endpoint() {
        let app = App::new(credentials_only_config());
        assert_eq!(app.state, LoadState::LoadingApis);
        assert_eq!(app.screen, Screen::EndpointPicker);
    }

    #[test]
    fn content_label_uses_schema_field_order_without_fixed_names() {
        let value = json!({"title": "T", "body": "B"});
        assert_eq!(content_label(&value, &["body".into(), "title".into()]), "B");
        assert_eq!(
            content_label(&json!({"id": "id1", "title": "T"}), &["summary".into()]),
            "id1"
        );
    }

    #[test]
    fn content_label_skips_empty_values_and_supports_other_json_values() {
        let value = json!({
            "emptyString": "",
            "nullValue": null,
            "emptyArray": [],
            "emptyObject": {},
            "number": 42,
            "flag": true
        });
        let order = vec![
            "emptyString".into(),
            "nullValue".into(),
            "emptyArray".into(),
            "emptyObject".into(),
            "number".into(),
            "flag".into(),
        ];
        assert_eq!(content_label(&value, &order), "42");
    }

    #[test]
    fn content_label_truncates_compact_json_to_80_characters() {
        let value = Value::String("x".repeat(100));
        let label = content_label(&value, &[]);
        assert_eq!(label.chars().count(), 80);
        assert!(label.ends_with("..."));
    }

    #[test]
    fn content_id_prefers_id_over_legacy_id() {
        let value = json!({"id": "primary", "_id": "legacy"});
        assert_eq!(content_id(&value), Some("primary"));
        assert_eq!(content_id(&json!({"_id": "legacy"})), Some("legacy"));
    }

    #[test]
    fn sanitized_payload_removes_read_only_fields() {
        let value = json!({
            "id": "content-id",
            "_id": "legacy-id",
            "createdAt": "created",
            "updatedAt": "updated",
            "publishedAt": "published",
            "revisedAt": "revised",
            "title": "Kept",
            "nested": {"id": "also-kept"}
        });
        assert_eq!(
            sanitized_payload(&value),
            json!({"title": "Kept", "nested": {"id": "also-kept"}})
        );
    }

    #[test]
    fn create_and_put_payload_remove_only_system_metadata_and_preserve_empty_fields() {
        let edited = json!({
            "id": "id1",
            "title": "",
            "image": "",
            "tags": [],
            "flag": null,
            "updatedAt": "read-only"
        });

        assert_eq!(
            sanitized_payload(&edited),
            json!({"title": "", "image": "", "tags": [], "flag": null})
        );
    }

    #[test]
    fn editable_payload_fills_missing_fields_and_removes_system_fields() {
        let template = json!({"title": "", "image": "", "tags": [], "flag": null});
        let content = json!({
            "id": "id1",
            "createdAt": "2026-01-01T00:00:00Z",
            "title": "Hello"
        });

        assert_eq!(
            editable_payload(Some(&template), &content),
            json!({"title": "Hello", "image": "", "tags": [], "flag": null})
        );
    }

    #[test]
    fn editable_payload_content_values_override_template_values() {
        assert_eq!(
            editable_payload(Some(&json!({"flag": null})), &json!({"flag": false})),
            json!({"flag": false})
        );
    }

    #[test]
    fn editable_payload_without_template_uses_sanitized_content() {
        let content = json!({"id": "id1", "title": "Existing"});
        assert_eq!(
            editable_payload(None, &content),
            json!({"title": "Existing"})
        );
        assert_eq!(
            editable_payload(Some(&Value::Null), &content),
            json!({"title": "Existing"})
        );
    }

    #[test]
    fn update_payload_uses_sanitized_payload_without_create_pruning() {
        let edited = json!({
            "id": "content-id",
            "title": "",
            "image": "",
            "tags": [],
            "flag": null
        });

        assert_eq!(
            sanitized_payload(&edited),
            json!({"title": "", "image": "", "tags": [], "flag": null})
        );
    }

    #[test]
    fn create_template_uses_official_api_fields_shape() {
        let schema = json!({
            "apiFields": [
                {"fieldId": "title", "kind": "text"},
                {"fieldId": "published", "kind": "boolean"},
                {"fieldId": "tags", "kind": "select", "multipleSelect": true},
                {"fieldId": "id", "kind": "text"}
            ]
        });
        assert_eq!(
            create_template_from_api_schema(&schema),
            Some(json!({"title": "", "published": null, "tags": []}))
        );
    }

    #[test]
    fn create_template_handles_nested_alternative_shape() {
        let schema = json!({
            "data": {
                "schema": {
                    "fields": [
                        {"id": "count", "fieldType": "number"},
                        {"key": "hero", "type": "media"},
                        {"key": "blocks", "type": "repeater"},
                        {"fieldId": "settings", "kind": "custom"}
                    ]
                }
            }
        });
        assert_eq!(
            create_template_from_api_schema(&schema),
            Some(json!({
                "count": null,
                "hero": "",
                "blocks": [],
                "settings": {}
            }))
        );
        assert_eq!(
            create_template_from_api_schema(&json!({"fields": []})),
            None
        );
    }

    #[test]
    fn create_template_uses_microcms_empty_values_for_known_kinds() {
        let schema = json!({
            "fields": [
                {"fieldId": "image", "kind": "image"},
                {"fieldId": "file", "kind": "file"},
                {"fieldId": "date", "kind": "date"},
                {"fieldId": "dateTime", "kind": "dateTime"},
                {"fieldId": "enabled", "kind": "boolean"},
                {"fieldId": "amount", "kind": "number"},
                {"fieldId": "reference", "kind": "contentReference"},
                {"fieldId": "custom", "kind": "custom"},
                {"fieldId": "extended", "kind": "extended"},
                {"fieldId": "object", "kind": "object"},
                {"fieldId": "images", "kind": "multipleImage"},
                {"fieldId": "references", "kind": "multipleContentReference"},
                {"fieldId": "repeat", "kind": "repeat"},
                {"fieldId": "array", "kind": "array"}
            ]
        });

        assert_eq!(
            create_template_from_api_schema(&schema),
            Some(json!({
                "image": "",
                "file": "",
                "date": "",
                "dateTime": "",
                "enabled": null,
                "amount": null,
                "reference": {},
                "custom": {},
                "extended": {},
                "object": {},
                "images": [],
                "references": [],
                "repeat": [],
                "array": []
            }))
        );
    }

    #[test]
    fn create_template_only_uses_string_for_known_single_select() {
        let schema = json!({
            "fields": [
                {"fieldId": "unknownMultiplicity", "kind": "select"},
                {"fieldId": "single", "kind": "select", "multipleSelect": false},
                {"fieldId": "multiple", "kind": "select", "multipleSelect": true}
            ]
        });

        assert_eq!(
            create_template_from_api_schema(&schema),
            Some(json!({
                "unknownMultiplicity": [],
                "single": "",
                "multiple": []
            }))
        );
    }

    #[test]
    fn create_template_uses_empty_strings_for_all_rich_editor_kinds() {
        let schema = json!({
            "apiFields": [
                {"fieldId": "modern", "kind": "richEditorV2"},
                {"fieldId": "current", "kind": "richEditor"},
                {"fieldId": "legacy", "kind": "oldRichEditor"}
            ]
        });

        assert_eq!(
            create_template_from_api_schema(&schema),
            Some(json!({"modern": "", "current": "", "legacy": ""}))
        );
    }

    #[test]
    fn schema_parser_preserves_api_fields_order_and_never_uses_name_as_key() {
        let schema = json!({
            "apiFields": [
                {"fieldId": "body", "name": "Body label", "kind": "textArea"},
                {"fieldId": "title", "name": "Title label", "kind": "text"},
                {"name": "Display name only", "kind": "text"},
                {"id": "legacy-id-in-apiFields", "kind": "text"}
            ]
        });

        assert_eq!(
            content_field_order_from_api_schema(&schema),
            vec!["body", "title"]
        );
        let template = create_template_from_api_schema(&schema).unwrap();
        assert_eq!(
            template
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["body", "title"]
        );
        assert!(template.get("Display name only").is_none());
        assert!(template.get("legacy-id-in-apiFields").is_none());
    }

    #[test]
    fn ordered_content_places_system_then_schema_then_other_fields() {
        let content = json!({
            "other": "last",
            "title": "Title",
            "revisedAt": "revised",
            "publishedAt": "published",
            "updatedAt": "updated",
            "createdAt": "created",
            "id": "content-id",
            "body": "Body"
        });
        let ordered = ordered_content_for_display(&content, &["body".into(), "title".into()], true);

        assert_eq!(
            ordered
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "id",
                "createdAt",
                "updatedAt",
                "publishedAt",
                "revisedAt",
                "body",
                "title",
                "other"
            ]
        );
    }

    #[test]
    fn edit_command_captures_id_and_opens_sanitized_value() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({
            "id": "content-id",
            "createdAt": "created",
            "title": "Editable"
        })];

        assert_eq!(
            app.apply_action(Action::Edit),
            Command::Update {
                content_id: "content-id".into(),
                value: json!({"title": "Editable"}),
                status: ContentWriteStatus::Default
            }
        );
        assert_eq!(app.items[0]["id"], "content-id");
    }

    #[test]
    fn edit_command_opens_schema_completed_value_but_uses_original_content_id() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.create_template = Some(json!({
            "title": "",
            "image": "",
            "tags": [],
            "flag": null
        }));
        app.items = vec![json!({
            "id": "content-id",
            "title": "Existing",
            "updatedAt": "read-only"
        })];

        assert_eq!(
            app.apply_action(Action::Edit),
            Command::Update {
                content_id: "content-id".into(),
                value: json!({
                    "title": "Existing",
                    "image": "",
                    "tags": [],
                    "flag": null
                }),
                status: ContentWriteStatus::Default
            }
        );
    }

    #[test]
    fn create_requires_stored_schema_template() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;

        assert_eq!(app.apply_action(Action::Create), Command::None);
        assert_eq!(
            app.message.as_deref(),
            Some("Schema unavailable; cannot create content.")
        );

        app.create_template = Some(json!({"headline": ""}));
        assert_eq!(
            app.apply_action(Action::Create),
            Command::Create {
                template: json!({"headline": ""}),
                status: ContentWriteStatus::Default
            }
        );
        assert_eq!(
            app.apply_action(Action::CreateDraft),
            Command::Create {
                template: json!({"headline": ""}),
                status: ContentWriteStatus::Draft
            }
        );
    }

    #[test]
    fn create_with_id_input_requires_id_and_schema_template() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;

        assert_eq!(app.apply_action(Action::CreateWithId), Command::None);
        assert_eq!(
            app.input_target,
            Some(InputTarget::CreateWithId(ContentWriteStatus::Default))
        );
        assert_eq!(app.apply_action(Action::InputApply), Command::None);
        assert_eq!(app.message.as_deref(), Some("Content ID is required."));

        app.apply_action(Action::CreateWithId);
        app.input_buffer = "content-id".into();
        assert_eq!(app.apply_action(Action::InputApply), Command::None);
        assert_eq!(
            app.message.as_deref(),
            Some("Schema unavailable; cannot create content.")
        );

        let template = create_template_from_api_schema(&json!({
            "apiFields": [
                {"fieldId": "body", "kind": "richEditorV2"},
                {"fieldId": "image", "kind": "image"}
            ]
        }))
        .unwrap();
        app.create_template = Some(template.clone());
        app.apply_action(Action::CreateWithId);
        app.input_buffer = "  specified-id  ".into();
        assert_eq!(
            app.apply_action(Action::InputApply),
            Command::CreateWithId {
                content_id: "specified-id".into(),
                template: template.clone(),
                status: ContentWriteStatus::Default
            }
        );

        app.apply_action(Action::CreateWithIdDraft);
        assert_eq!(
            app.input_target,
            Some(InputTarget::CreateWithId(ContentWriteStatus::Draft))
        );
        app.input_buffer = "draft-id".into();
        assert_eq!(
            app.apply_action(Action::InputApply),
            Command::CreateWithId {
                content_id: "draft-id".into(),
                template,
                status: ContentWriteStatus::Draft
            }
        );
    }

    #[test]
    fn fullscreen_preview_toggles_scrolls_and_moves_within_page() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "first"}), json!({"id": "second"})];

        assert_eq!(
            app.apply_action(Action::TogglePreviewFullscreen),
            Command::None
        );
        assert!(app.preview_fullscreen);
        assert_eq!(app.preview_scroll, 0);

        app.apply_action(Action::PreviewScrollDown);
        assert_eq!(app.preview_scroll, 1);
        app.apply_action(Action::PreviewScrollUp);
        assert_eq!(app.preview_scroll, 0);
        app.apply_action(Action::PreviewScrollBottom);
        assert_eq!(app.preview_scroll, u16::MAX);
        app.apply_action(Action::PreviewScrollTop);
        assert_eq!(app.preview_scroll, 0);

        app.preview_scroll = 10;
        app.apply_action(Action::PreviewNextContent);
        assert_eq!(app.content_selected, 1);
        assert_eq!(app.preview_scroll, 0);
        app.apply_action(Action::PreviewNextContent);
        assert_eq!(app.content_selected, 1);
        app.apply_action(Action::PreviewPrevContent);
        assert_eq!(app.content_selected, 0);
        app.apply_action(Action::PreviewPrevContent);
        assert_eq!(app.content_selected, 0);

        app.preview_scroll = 5;
        app.apply_action(Action::ClosePreviewFullscreen);
        assert!(!app.preview_fullscreen);
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn fullscreen_preview_crud_and_status_actions_use_current_content() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.preview_fullscreen = true;
        app.items = vec![json!({"id": "content-id", "body": "Body"})];

        assert_eq!(
            app.apply_action(Action::Edit),
            Command::Update {
                content_id: "content-id".into(),
                value: json!({"body": "Body"}),
                status: ContentWriteStatus::Default
            }
        );
        assert_eq!(
            app.apply_action(Action::EditDraft),
            Command::Update {
                content_id: "content-id".into(),
                value: json!({"body": "Body"}),
                status: ContentWriteStatus::Draft
            }
        );
        assert_eq!(app.apply_action(Action::DeleteRequest), Command::None);
        assert_eq!(
            app.pending_confirmation,
            Some(PendingConfirmation::Delete {
                content_ids: vec!["content-id".into()]
            })
        );
        app.apply_action(Action::CancelPending);
        assert!(app.preview_fullscreen);
        assert_eq!(app.apply_action(Action::Publish), Command::None);
        assert!(matches!(
            app.pending_confirmation,
            Some(PendingConfirmation::PublicationStatus {
                status: PublicationStatus::Publish,
                ..
            })
        ));
        app.apply_action(Action::CancelPending);
        assert_eq!(app.apply_action(Action::Draft), Command::None);
        assert!(matches!(
            app.pending_confirmation,
            Some(PendingConfirmation::PublicationStatus {
                status: PublicationStatus::Draft,
                ..
            })
        ));
        app.apply_action(Action::CancelPending);
        app.apply_action(Action::DeleteRequest);
        assert_eq!(
            app.apply_action(Action::ConfirmPending),
            Command::Confirmed(PendingConfirmation::Delete {
                content_ids: vec!["content-id".into()]
            })
        );
        assert!(!app.preview_fullscreen);
    }

    #[test]
    fn content_context_changes_close_fullscreen_preview() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.endpoint = Some("blogs".into());
        app.items = vec![json!({"id": "content-id"})];
        app.preview_fullscreen = true;
        app.preview_scroll = 8;

        assert_eq!(app.apply_action(Action::Reload), Command::FetchContents);
        assert!(!app.preview_fullscreen);
        assert_eq!(app.preview_scroll, 0);

        app.state = LoadState::ContentsLoaded;
        app.preview_fullscreen = true;
        assert_eq!(app.apply_action(Action::NextPage), Command::FetchContents);
        assert!(!app.preview_fullscreen);

        app.state = LoadState::ContentsLoaded;
        app.preview_fullscreen = true;
        app.input_target = Some(InputTarget::Search);
        app.input_buffer = "query".into();
        assert_eq!(app.apply_action(Action::InputApply), Command::FetchContents);
        assert!(!app.preview_fullscreen);

        app.preview_fullscreen = true;
        assert_eq!(
            app.apply_event(AppEvent::MutationSucceeded {
                endpoint: "blogs".into(),
                message: "Created.".into(),
            }),
            Command::FetchContents
        );
        assert!(!app.preview_fullscreen);

        app.preview_fullscreen = true;
        app.apply_event(AppEvent::ContentsLoaded {
            endpoint: "blogs".into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::List,
                total_count: 1,
                offset: 0,
                limit: PAGE_LIMIT,
                contents: vec![json!({"id": "replacement"})],
            },
            statuses: HashMap::new(),
            status_warning: None,
            create_template: None,
            content_field_order: None,
            schema_warning: None,
        });
        assert!(!app.preview_fullscreen);

        app.screen = Screen::EndpointPicker;
        app.state = LoadState::ApisLoaded;
        app.apis = vec![ApiInfo {
            endpoint: "news".into(),
            name: None,
            description: None,
        }];
        app.preview_fullscreen = true;
        assert_eq!(app.apply_action(Action::Select), Command::FetchContents);
        assert!(!app.preview_fullscreen);
    }

    #[test]
    fn delete_confirmation_can_be_cancelled_or_confirmed() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.endpoint = Some("blogs".into());
        app.state = LoadState::ContentsLoaded;
        app.items = vec![
            json!({"id": "first", "title": "First"}),
            json!({"id": "second", "title": "Second"}),
        ];

        assert_eq!(app.apply_action(Action::DeleteRequest), Command::None);
        assert_eq!(
            app.pending_confirmation,
            Some(PendingConfirmation::Delete {
                content_ids: vec!["first".into()]
            })
        );
        assert_eq!(app.apply_action(Action::CancelPending), Command::None);
        assert!(app.pending_confirmation.is_none());

        app.selected_content_ids.insert("first".into());
        app.selected_content_ids.insert("second".into());
        app.apply_action(Action::DeleteRequest);
        assert_eq!(
            app.apply_action(Action::ConfirmPending),
            Command::Confirmed(PendingConfirmation::Delete {
                content_ids: vec!["first".into(), "second".into()]
            })
        );
        assert!(app.pending_confirmation.is_none());
    }

    #[test]
    fn toggle_select_adds_and_removes_current_content_id() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "content-id"})];

        assert_eq!(app.apply_action(Action::ToggleSelect), Command::None);
        assert!(app.selected_content_ids.contains("content-id"));
        assert_eq!(app.apply_action(Action::ToggleSelect), Command::None);
        assert!(app.selected_content_ids.is_empty());
    }

    #[test]
    fn toggle_select_rejects_content_without_id() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"title": "No ID"})];

        assert_eq!(app.apply_action(Action::ToggleSelect), Command::None);
        assert!(app.selected_content_ids.is_empty());
        assert_eq!(
            app.message.as_deref(),
            Some("Selected content has no id or _id; cannot select.")
        );
    }

    #[test]
    fn clear_query_resets_all_query_fields_and_offset() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.search_query = Some("keyword".into());
        app.filters = Some("category[equals]news".into());
        app.orders = Some("-publishedAt".into());
        app.offset = 40;
        app.selected_content_ids.insert("selected".into());

        assert_eq!(app.apply_action(Action::ClearQuery), Command::FetchContents);
        assert_eq!(app.search_query, None);
        assert_eq!(app.filters, None);
        assert_eq!(app.orders, None);
        assert_eq!(app.offset, 0);
        assert!(app.selected_content_ids.is_empty());
    }

    #[test]
    fn publish_and_draft_require_confirmation_and_use_selected_or_current_ids() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "first"}), json!({"id": "second"})];

        assert_eq!(app.apply_action(Action::Publish), Command::None);
        assert_eq!(
            app.pending_confirmation,
            Some(PendingConfirmation::PublicationStatus {
                content_ids: vec!["first".into()],
                status: PublicationStatus::Publish
            })
        );
        assert!(matches!(
            app.apply_action(Action::ConfirmPending),
            Command::Confirmed(PendingConfirmation::PublicationStatus { .. })
        ));

        app.selected_content_ids.insert("first".into());
        app.selected_content_ids.insert("second".into());
        assert_eq!(app.apply_action(Action::Draft), Command::None);
        assert_eq!(
            app.pending_confirmation,
            Some(PendingConfirmation::PublicationStatus {
                content_ids: vec!["first".into(), "second".into()],
                status: PublicationStatus::Draft
            })
        );
    }

    #[test]
    fn search_input_applies_trimmed_value_and_resets_offset() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.offset = 40;
        app.selected_content_ids.insert("selected".into());

        assert_eq!(app.apply_action(Action::EditSearch), Command::None);
        app.apply_action(Action::InputChar(' '));
        app.apply_action(Action::InputChar('r'));
        app.apply_action(Action::InputChar('u'));
        app.apply_action(Action::InputChar('s'));
        app.apply_action(Action::InputChar('t'));
        app.apply_action(Action::InputChar(' '));
        assert_eq!(app.apply_action(Action::InputApply), Command::FetchContents);
        assert_eq!(app.search_query.as_deref(), Some("rust"));
        assert_eq!(app.offset, 0);
        assert_eq!(app.input_target, None);
        assert!(app.selected_content_ids.is_empty());
    }

    #[test]
    fn cancelling_input_preserves_existing_query() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.search_query = Some("existing".into());

        app.apply_action(Action::EditSearch);
        app.apply_action(Action::InputChar('x'));
        assert_eq!(app.apply_action(Action::InputCancel), Command::None);
        assert_eq!(app.search_query.as_deref(), Some("existing"));
        assert_eq!(app.input_target, None);
    }

    #[test]
    fn status_and_crud_successes_reload_contents() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.endpoint = Some("blogs".into());
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "content-id"})];
        app.content_selected = 0;
        app.offset = 20;

        assert_eq!(
            app.apply_event(AppEvent::StatusSucceeded {
                endpoint: "blogs".into(),
                message: "Content set to draft.".into(),
            }),
            Command::FetchContents
        );
        assert_eq!(app.items, vec![json!({"id": "content-id"})]);
        assert_eq!(app.offset, 20);
        assert_eq!(app.state, LoadState::LoadingContents);

        assert_eq!(
            app.apply_event(AppEvent::MutationSucceeded {
                endpoint: "blogs".into(),
                message: "Content updated.".into(),
            }),
            Command::FetchContents
        );
    }

    #[test]
    fn publication_state_maps_management_status_arrays() {
        assert_eq!(
            content_publication_state(&["PUBLISH".into()]),
            ContentPublicationState::Published
        );
        assert_eq!(
            content_publication_state(&["DRAFT".into()]),
            ContentPublicationState::Draft
        );
        assert_eq!(
            content_publication_state(&["PUBLISH".into(), "DRAFT".into()]),
            ContentPublicationState::PublishedAndDraft
        );
        assert_eq!(
            content_publication_state(&["PUBLISH_AND_DRAFT".into()]),
            ContentPublicationState::PublishedAndDraft
        );
        assert_eq!(
            content_publication_state(&["CLOSED".into()]),
            ContentPublicationState::Closed
        );
        assert_eq!(
            content_publication_state(&[]),
            ContentPublicationState::Unknown
        );
        assert_eq!(
            content_publication_state(&["OTHER".into()]),
            ContentPublicationState::Unknown
        );
    }

    #[test]
    fn publication_state_for_content_merges_by_id_and_defaults_unknown() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("blogs".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::LoadingContents;
        app.selected_content_ids.insert("stale-selection".into());
        let statuses = HashMap::from([("known".into(), ContentPublicationState::Draft)]);
        app.apply_event(AppEvent::ContentsLoaded {
            endpoint: "blogs".into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::List,
                total_count: 2,
                offset: 0,
                limit: PAGE_LIMIT,
                contents: vec![json!({"id": "known"}), json!({"id": "missing"})],
            },
            statuses,
            status_warning: Some("Some status metadata is missing.".into()),
            create_template: None,
            content_field_order: None,
            schema_warning: None,
        });

        assert_eq!(
            app.publication_state_for(&app.items[0]),
            ContentPublicationState::Draft
        );
        assert_eq!(
            app.publication_state_for(&app.items[1]),
            ContentPublicationState::Unknown
        );
        assert_eq!(
            app.publication_state_for(&json!({"title": "No ID"})),
            ContentPublicationState::Unknown
        );
        assert!(app.selected_content_ids.is_empty());
        assert_eq!(
            app.message.as_deref(),
            Some("Some status metadata is missing.")
        );
    }

    #[test]
    fn help_actions_preserve_delete_state_and_context_changes_close_help() {
        let mut app = App::new(credentials_only_config());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.endpoint = Some("blogs".into());
        app.pending_confirmation = Some(PendingConfirmation::Delete {
            content_ids: vec!["content-id".into()],
        });

        assert_eq!(app.apply_action(Action::ToggleHelp), Command::None);
        assert!(app.help_open);
        assert!(app.pending_confirmation.is_some());
        assert_eq!(app.apply_action(Action::CloseHelp), Command::None);
        assert!(!app.help_open);
        assert!(app.pending_confirmation.is_some());

        app.pending_confirmation = None;
        app.help_open = true;
        assert_eq!(app.apply_action(Action::Reload), Command::FetchContents);
        assert!(!app.help_open);
    }

    #[test]
    fn object_api_allows_preview_but_blocks_all_list_only_operations() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("object-endpoint".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::LoadingContents;
        app.apply_event(AppEvent::ContentsLoaded {
            endpoint: "object-endpoint".into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::Object,
                total_count: 1,
                offset: 0,
                limit: 1,
                contents: vec![json!({"id": "object-id", "body": "Object body"})],
            },
            statuses: HashMap::from([("object-id".into(), ContentPublicationState::Published)]),
            status_warning: Some("must be ignored for object APIs".into()),
            create_template: Some(json!({"body": ""})),
            content_field_order: Some(vec!["body".into()]),
            schema_warning: None,
        });

        assert_eq!(app.content_kind, ContentCollectionKind::Object);
        assert!(app.content_statuses.is_empty());
        for action in [
            Action::NextPage,
            Action::PrevPage,
            Action::ToggleSelect,
            Action::Create,
            Action::CreateDraft,
            Action::CreateWithId,
            Action::CreateWithIdDraft,
            Action::Edit,
            Action::EditDraft,
            Action::DeleteRequest,
            Action::Publish,
            Action::Draft,
        ] {
            assert_eq!(app.apply_action(action), Command::None, "{action:?}");
            assert!(app.selected_content_ids.is_empty(), "{action:?}");
            assert!(app.pending_confirmation.is_none(), "{action:?}");
            assert_eq!(app.input_target, None, "{action:?}");
        }

        assert_eq!(
            app.apply_action(Action::TogglePreviewFullscreen),
            Command::None
        );
        assert!(app.preview_fullscreen);
    }

    #[test]
    fn selecting_list_endpoint_after_object_resets_page_limit() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("object-endpoint".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::LoadingContents;
        app.apply_event(AppEvent::ContentsLoaded {
            endpoint: "object-endpoint".into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::Object,
                total_count: 1,
                offset: 0,
                limit: 1,
                contents: vec![json!({"body": "Object body"})],
            },
            statuses: HashMap::new(),
            status_warning: None,
            create_template: None,
            content_field_order: None,
            schema_warning: None,
        });
        assert_eq!(app.limit, 1);
        assert_eq!(app.content_kind, ContentCollectionKind::Object);

        assert_eq!(app.apply_action(Action::Back), Command::None);
        app.apis = vec![ApiInfo {
            endpoint: "list-endpoint".into(),
            name: None,
            description: None,
        }];
        app.api_selected = 0;

        assert_eq!(app.apply_action(Action::Select), Command::FetchContents);
        assert_eq!(app.limit, PAGE_LIMIT);
        assert_eq!(app.offset, 0);
        assert_eq!(app.content_kind, ContentCollectionKind::List);
        assert!(app.items.is_empty());
    }

    #[test]
    fn stale_contents_response_is_a_complete_no_op() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("current".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::LoadingContents;
        app.items = vec![json!({"id": "current-item"})];
        app.create_template = Some(json!({"current": ""}));
        app.content_field_order = vec!["current".into()];
        app.content_kind = ContentCollectionKind::Object;
        app.content_statuses =
            HashMap::from([("current-item".into(), ContentPublicationState::Draft)]);
        app.limit = 7;
        app.message = Some("keep this message".into());
        app.help_open = true;
        app.preview_fullscreen = true;
        app.preview_scroll = 9;

        let stale_event = AppEvent::ContentsLoaded {
            endpoint: "stale".into(),
            collection: ContentCollection {
                kind: ContentCollectionKind::List,
                total_count: 99,
                offset: 20,
                limit: PAGE_LIMIT,
                contents: vec![json!({"id": "stale-item"})],
            },
            statuses: HashMap::from([("stale-item".into(), ContentPublicationState::Published)]),
            status_warning: Some("stale warning".into()),
            create_template: Some(json!({"stale": ""})),
            content_field_order: Some(vec!["stale".into()]),
            schema_warning: Some("stale schema warning".into()),
        };

        assert_eq!(app.apply_event(stale_event), Command::None);
        assert_eq!(app.items, vec![json!({"id": "current-item"})]);
        assert_eq!(app.create_template, Some(json!({"current": ""})));
        assert_eq!(app.content_field_order, vec!["current"]);
        assert_eq!(app.content_kind, ContentCollectionKind::Object);
        assert_eq!(
            app.content_statuses.get("current-item"),
            Some(&ContentPublicationState::Draft)
        );
        assert_eq!(app.limit, 7);
        assert_eq!(app.message.as_deref(), Some("keep this message"));
        assert!(app.preview_fullscreen);
        assert!(app.help_open);
        assert_eq!(app.preview_scroll, 9);
        assert_eq!(app.state, LoadState::LoadingContents);

        assert_eq!(
            app.apply_event(AppEvent::ContentsLoaded {
                endpoint: "current".into(),
                collection: ContentCollection {
                    kind: ContentCollectionKind::List,
                    total_count: 1,
                    offset: 0,
                    limit: PAGE_LIMIT,
                    contents: vec![json!({"id": "fresh-item"})],
                },
                statuses: HashMap::new(),
                status_warning: None,
                create_template: Some(json!({"fresh": ""})),
                content_field_order: Some(vec!["fresh".into()]),
                schema_warning: None,
            }),
            Command::None
        );
        assert_eq!(app.items, vec![json!({"id": "fresh-item"})]);
        assert_eq!(app.content_kind, ContentCollectionKind::List);
        assert_eq!(app.limit, PAGE_LIMIT);
        assert_eq!(app.state, LoadState::ContentsLoaded);
        assert!(!app.preview_fullscreen);
    }

    #[test]
    fn contents_response_requires_browser_loading_state_and_clears_input_when_applied() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("current".into());
        app.items = vec![json!({"id": "existing"})];

        app.screen = Screen::EndpointPicker;
        app.state = LoadState::ApisLoaded;
        assert_eq!(
            app.apply_event(contents_loaded_event("current", "picker-response")),
            Command::None
        );
        assert_eq!(app.items, vec![json!({"id": "existing"})]);
        assert_eq!(app.screen, Screen::EndpointPicker);

        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        assert_eq!(
            app.apply_event(contents_loaded_event("current", "idle-response")),
            Command::None
        );
        assert_eq!(app.items, vec![json!({"id": "existing"})]);
        assert_eq!(app.state, LoadState::ContentsLoaded);

        app.state = LoadState::LoadingContents;
        app.input_target = Some(InputTarget::Search);
        app.input_buffer = "unfinished query".into();
        assert_eq!(
            app.apply_event(contents_loaded_event("current", "fresh")),
            Command::None
        );
        assert_eq!(app.items, vec![json!({"id": "fresh"})]);
        assert_eq!(app.input_target, None);
        assert!(app.input_buffer.is_empty());
        assert_eq!(app.state, LoadState::ContentsLoaded);
    }

    #[test]
    fn fetch_failures_apply_only_to_matching_active_fetch() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("current".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::LoadingContents;

        assert_eq!(
            app.apply_event(AppEvent::FetchFailed {
                endpoint: Some("stale".into()),
                error: "stale failure".into(),
            }),
            Command::None
        );
        assert_eq!(app.state, LoadState::LoadingContents);

        assert_eq!(
            app.apply_event(AppEvent::FetchFailed {
                endpoint: Some("current".into()),
                error: "current failure".into(),
            }),
            Command::None
        );
        assert_eq!(app.state, LoadState::Error("current failure".into()));

        let mut api_app = App::new(credentials_only_config());
        assert_eq!(api_app.screen, Screen::EndpointPicker);
        assert_eq!(api_app.state, LoadState::LoadingApis);
        api_app.apply_event(AppEvent::FetchFailed {
            endpoint: None,
            error: "API discovery failed".into(),
        });
        assert_eq!(
            api_app.state,
            LoadState::Error("API discovery failed".into())
        );

        api_app.state = LoadState::LoadingApis;
        api_app.screen = Screen::ContentBrowser;
        api_app.apply_event(AppEvent::FetchFailed {
            endpoint: None,
            error: "late API failure".into(),
        });
        assert_eq!(api_app.state, LoadState::LoadingApis);
    }

    #[test]
    fn mutation_events_apply_only_to_matching_content_browser() {
        let mut app = App::new(credentials_only_config());
        app.endpoint = Some("current".into());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.message = Some("unchanged".into());

        assert_eq!(
            app.apply_event(AppEvent::MutationSucceeded {
                endpoint: "stale".into(),
                message: "stale success".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("unchanged"));
        assert_eq!(app.state, LoadState::ContentsLoaded);

        assert_eq!(
            app.apply_event(AppEvent::StatusSucceeded {
                endpoint: "stale".into(),
                message: "stale status".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("unchanged"));

        assert_eq!(
            app.apply_event(AppEvent::MutationFailed {
                endpoint: "stale".into(),
                error: "stale failure".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("unchanged"));

        app.screen = Screen::EndpointPicker;
        assert_eq!(
            app.apply_event(AppEvent::MutationFailed {
                endpoint: "current".into(),
                error: "late failure".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("unchanged"));
        assert_eq!(
            app.apply_event(AppEvent::MutationSucceeded {
                endpoint: "current".into(),
                message: "late success".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("unchanged"));

        app.screen = Screen::ContentBrowser;
        assert_eq!(
            app.apply_event(AppEvent::MutationSucceeded {
                endpoint: "current".into(),
                message: "saved".into(),
            }),
            Command::FetchContents
        );
        assert_eq!(app.state, LoadState::LoadingContents);
        assert_eq!(app.message.as_deref(), Some("saved"));

        app.state = LoadState::ContentsLoaded;
        assert_eq!(
            app.apply_event(AppEvent::StatusSucceeded {
                endpoint: "current".into(),
                message: "published".into(),
            }),
            Command::FetchContents
        );
        assert!(app.message.as_deref().unwrap().starts_with("published"));

        app.state = LoadState::ContentsLoaded;
        assert_eq!(
            app.apply_event(AppEvent::MutationFailed {
                endpoint: "current".into(),
                error: "request failed".into(),
            }),
            Command::None
        );
        assert_eq!(app.message.as_deref(), Some("error: request failed"));
    }
}
