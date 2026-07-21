use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    app::{
        content_field_order_from_api_schema, content_id, content_publication_state,
        create_template_from_api_schema, sanitized_payload, Action, App, AppEvent, Command,
        LoadState, PendingConfirmation, Screen,
    },
    config::Config,
    microcms::{
        ContentCollectionKind, ContentQuery, ContentWriteStatus, MicroCmsClient, PublicationStatus,
    },
    ui,
};

type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

enum MutationRequest {
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
    Delete {
        content_ids: Vec<String>,
    },
    Status {
        content_ids: Vec<String>,
        status: PublicationStatus,
    },
}

impl MutationRequest {
    fn into_write_confirmation(self) -> Result<PendingConfirmation, Self> {
        match self {
            Self::Create {
                value,
                status: ContentWriteStatus::Default,
            } => Ok(PendingConfirmation::Create {
                value,
                status: ContentWriteStatus::Default,
            }),
            Self::PutCreate {
                content_id,
                value,
                status: ContentWriteStatus::Default,
            } => Ok(PendingConfirmation::PutCreate {
                content_id,
                value,
                status: ContentWriteStatus::Default,
            }),
            Self::Update {
                content_id,
                value,
                status: ContentWriteStatus::Default,
            } => Ok(PendingConfirmation::Update {
                content_id,
                value,
                status: ContentWriteStatus::Default,
            }),
            mutation => Err(mutation),
        }
    }
}

#[derive(Debug, PartialEq)]
enum EditResult {
    Changed(Value),
    Unchanged,
}

pub fn run(config: Config) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let result = run_loop(&mut terminal, config);
    let restore_result = restore_terminal(&mut terminal);

    match (result, restore_result) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn setup_terminal() -> Result<TuiTerminal> {
    enable_raw_mode().context("failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error).context("failed to enter alternate screen");
    }
    match Terminal::new(CrosstermBackend::new(stdout)) {
        Ok(mut terminal) => {
            if let Err(error) = terminal.hide_cursor() {
                let _ = restore_terminal(&mut terminal);
                return Err(error).context("failed to hide cursor");
            }
            Ok(terminal)
        }
        Err(error) => {
            let mut stdout = io::stdout();
            let _ = execute!(stdout, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            Err(error).context("failed to initialize terminal")
        }
    }
}

fn restore_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    let raw_result = disable_raw_mode().context("failed to disable terminal raw mode");
    let screen_result = execute!(terminal.backend_mut(), LeaveAlternateScreen)
        .context("failed to leave alternate screen");
    let cursor_result = terminal.show_cursor().context("failed to show cursor");
    raw_result.and(screen_result).and(cursor_result)
}

fn resume_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    enable_raw_mode().context("failed to re-enable terminal raw mode")?;
    if let Err(error) = execute!(terminal.backend_mut(), EnterAlternateScreen) {
        let _ = disable_raw_mode();
        return Err(error).context("failed to re-enter alternate screen");
    }
    terminal
        .hide_cursor()
        .context("failed to hide cursor after editor")?;
    terminal.clear().context("failed to redraw after editor")
}

fn run_loop(terminal: &mut TuiTerminal, config: Config) -> Result<()> {
    let mut app = App::new(config);
    let (tx, mut rx) = mpsc::unbounded_channel();

    if matches!(app.state, LoadState::LoadingApis) {
        schedule_fetch(&app, Command::FetchApis, tx.clone());
    }

    while !app.should_quit {
        while let Ok(app_event) = rx.try_recv() {
            let command = app.apply_event(app_event);
            handle_command(terminal, &mut app, command, tx.clone());
        }

        terminal.draw(|frame| ui::draw(frame, &app))?;

        if event::poll(Duration::from_millis(50)).context("failed to poll terminal events")? {
            if let Event::Key(key) = event::read().context("failed to read terminal event")? {
                if !matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) {
                    continue;
                }
                if let Some(action) = action_for_key(key, &app) {
                    let command = app.apply_action(action);
                    handle_command(terminal, &mut app, command, tx.clone());
                }
            }
        }
    }
    Ok(())
}

fn action_for_key(key: KeyEvent, app: &App) -> Option<Action> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Some(Action::Quit);
    }
    let code = key.code;
    if app.help_open {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Esc | KeyCode::Enter => Some(Action::CloseHelp),
            _ => None,
        };
    }
    if app.input_target.is_some() {
        return match code {
            KeyCode::Char(character) => Some(Action::InputChar(character)),
            KeyCode::Backspace => Some(Action::InputBackspace),
            KeyCode::Enter => Some(Action::InputApply),
            KeyCode::Esc => Some(Action::InputCancel),
            _ => None,
        };
    }
    if app.pending_confirmation.is_some() {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('y') => Some(Action::ConfirmPending),
            KeyCode::Char('n') | KeyCode::Esc => Some(Action::CancelPending),
            _ => None,
        };
    }
    if app.preview_fullscreen {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Enter | KeyCode::Esc => Some(Action::ClosePreviewFullscreen),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::PreviewScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::PreviewScrollUp),
            KeyCode::Char('g') => Some(Action::PreviewScrollTop),
            KeyCode::Char('G') => Some(Action::PreviewScrollBottom),
            KeyCode::Char('n') | KeyCode::PageDown => Some(Action::PreviewNextContent),
            KeyCode::Char('p') | KeyCode::PageUp => Some(Action::PreviewPrevContent),
            KeyCode::Char('e') => Some(Action::Edit),
            KeyCode::Char('E') => Some(Action::EditDraft),
            KeyCode::Char('d') => Some(Action::DeleteRequest),
            KeyCode::Char('P') => Some(Action::Publish),
            KeyCode::Char('D') => Some(Action::Draft),
            _ => None,
        };
    }

    match code {
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        KeyCode::Esc if app.screen == Screen::ContentBrowser => Some(Action::Back),
        KeyCode::Esc => Some(Action::Quit),
        KeyCode::Char('b') if app.screen == Screen::ContentBrowser => Some(Action::Back),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Char(' ') if app.screen == Screen::ContentBrowser => Some(Action::ToggleSelect),
        KeyCode::Enter if app.screen == Screen::EndpointPicker => Some(Action::Select),
        KeyCode::Enter if app.screen == Screen::ContentBrowser => {
            Some(Action::TogglePreviewFullscreen)
        }
        KeyCode::Char('r') => Some(Action::Reload),
        KeyCode::Char('c') if app.screen == Screen::ContentBrowser => Some(Action::Create),
        KeyCode::Char('C') if app.screen == Screen::ContentBrowser => Some(Action::CreateDraft),
        KeyCode::Char('u') if app.screen == Screen::ContentBrowser => Some(Action::CreateWithId),
        KeyCode::Char('U') if app.screen == Screen::ContentBrowser => {
            Some(Action::CreateWithIdDraft)
        }
        KeyCode::Char('e') if app.screen == Screen::ContentBrowser => Some(Action::Edit),
        KeyCode::Char('E') if app.screen == Screen::ContentBrowser => Some(Action::EditDraft),
        KeyCode::Char('d') if app.screen == Screen::ContentBrowser => Some(Action::DeleteRequest),
        KeyCode::Char('/') if app.screen == Screen::ContentBrowser => Some(Action::EditSearch),
        KeyCode::Char('f') if app.screen == Screen::ContentBrowser => Some(Action::EditFilters),
        KeyCode::Char('o') if app.screen == Screen::ContentBrowser => Some(Action::EditOrders),
        KeyCode::Char('x') if app.screen == Screen::ContentBrowser => Some(Action::ClearQuery),
        KeyCode::Char('P') if app.screen == Screen::ContentBrowser => Some(Action::Publish),
        KeyCode::Char('D') if app.screen == Screen::ContentBrowser => Some(Action::Draft),
        KeyCode::Char('n') | KeyCode::PageDown if app.screen == Screen::ContentBrowser => {
            Some(Action::NextPage)
        }
        KeyCode::Char('p') | KeyCode::PageUp if app.screen == Screen::ContentBrowser => {
            Some(Action::PrevPage)
        }
        _ => None,
    }
}

fn handle_command(
    terminal: &mut TuiTerminal,
    app: &mut App,
    command: Command,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    match command {
        Command::None => {}
        fetch @ (Command::FetchApis | Command::FetchContents) => schedule_fetch(app, fetch, tx),
        Command::Create { template, status } => match edit_json(terminal, "create", &template) {
            Ok(EditResult::Changed(value)) => {
                let value = sanitized_payload(&value);
                queue_write_mutation(app, MutationRequest::Create { value, status }, tx);
            }
            Ok(EditResult::Unchanged) => {
                app.message = Some("Create cancelled; no changes.".into());
            }
            Err(error) => app.message = Some(format!("error: {error:#}")),
        },
        Command::CreateWithId {
            content_id,
            template,
            status,
        } => match edit_json(terminal, "put-create", &template) {
            Ok(EditResult::Changed(value)) => {
                let value = sanitized_payload(&value);
                queue_write_mutation(
                    app,
                    MutationRequest::PutCreate {
                        content_id,
                        value,
                        status,
                    },
                    tx,
                );
            }
            Ok(EditResult::Unchanged) => {
                app.message = Some("Create with ID cancelled; no changes.".into());
            }
            Err(error) => app.message = Some(format!("error: {error:#}")),
        },
        Command::Update {
            content_id,
            value,
            status,
        } => match edit_json(terminal, "update", &value) {
            Ok(EditResult::Changed(value)) => {
                let value = sanitized_payload(&value);
                queue_write_mutation(
                    app,
                    MutationRequest::Update {
                        content_id,
                        value,
                        status,
                    },
                    tx,
                );
            }
            Ok(EditResult::Unchanged) => {
                app.message = Some("Update cancelled; no changes.".into());
            }
            Err(error) => app.message = Some(format!("error: {error:#}")),
        },
        Command::Confirmed(confirmation) => {
            schedule_confirmed_mutation(app, confirmation, tx);
        }
    }
}

fn queue_write_mutation(
    app: &mut App,
    mutation: MutationRequest,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    match mutation.into_write_confirmation() {
        Ok(confirmation) => app.request_confirmation(confirmation),
        Err(mutation) => {
            app.message = Some(match &mutation {
                MutationRequest::Create { .. } => "Creating draft content...".into(),
                MutationRequest::PutCreate { content_id, .. } => {
                    format!("Creating draft content with ID {content_id}...")
                }
                MutationRequest::Update { .. } => "Updating draft content...".into(),
                MutationRequest::Delete { .. } | MutationRequest::Status { .. } => {
                    unreachable!("only content writes are queued here")
                }
            });
            schedule_mutation(app, mutation, tx);
        }
    }
}

fn schedule_confirmed_mutation(
    app: &mut App,
    confirmation: PendingConfirmation,
    tx: mpsc::UnboundedSender<AppEvent>,
) {
    let mutation = match confirmation {
        PendingConfirmation::Delete { content_ids } => {
            app.message = Some(if content_ids.len() == 1 {
                "Deleting content...".into()
            } else {
                format!("Deleting {} contents...", content_ids.len())
            });
            MutationRequest::Delete { content_ids }
        }
        PendingConfirmation::Create { value, status } => {
            app.message = Some("Creating content...".into());
            MutationRequest::Create { value, status }
        }
        PendingConfirmation::PutCreate {
            content_id,
            value,
            status,
        } => {
            app.message = Some(format!("Creating content with ID {content_id}..."));
            MutationRequest::PutCreate {
                content_id,
                value,
                status,
            }
        }
        PendingConfirmation::Update {
            content_id,
            value,
            status,
        } => {
            app.message = Some("Updating content...".into());
            MutationRequest::Update {
                content_id,
                value,
                status,
            }
        }
        PendingConfirmation::PublicationStatus {
            content_ids,
            status,
        } => {
            let count = content_ids.len();
            app.message = Some(match (status, count) {
                (PublicationStatus::Publish, 1) => "Publishing content...".into(),
                (PublicationStatus::Publish, count) => format!("Publishing {count} contents..."),
                (PublicationStatus::Draft, 1) => "Setting content to draft...".into(),
                (PublicationStatus::Draft, count) => {
                    format!("Setting {count} contents to draft...")
                }
            });
            MutationRequest::Status {
                content_ids,
                status,
            }
        }
    };
    schedule_mutation(app, mutation, tx);
}

fn schema_load_result(schema: Value) -> (Option<Value>, Option<Vec<String>>, Option<String>) {
    let field_order = content_field_order_from_api_schema(&schema);
    match create_template_from_api_schema(&schema) {
        Some(template) => (Some(template), Some(field_order), None),
        None => (
            None,
            Some(field_order),
            Some(
                "Schema unavailable; no user-defined fields could be parsed, so create is disabled."
                    .to_string(),
            ),
        ),
    }
}

fn schedule_fetch(app: &App, command: Command, tx: mpsc::UnboundedSender<AppEvent>) {
    let service_id = app.config.service_id.clone();
    let api_key = app.config.api_key.clone();
    let endpoint = app.endpoint.clone();
    let offset = app.offset;
    let limit = app.limit;
    let query = ContentQuery {
        q: app.search_query.clone(),
        filters: app.filters.clone(),
        orders: app.orders.clone(),
    };
    let needs_schema = app.create_template.is_none();
    let failure_endpoint = match &command {
        Command::FetchContents => endpoint.clone(),
        _ => None,
    };

    tokio::spawn(async move {
        let result: Result<AppEvent> = async {
            let service_id = service_id.context("service ID is missing")?;
            let api_key = api_key.context("API key is missing")?;
            let client = MicroCmsClient::new(service_id, api_key)?;

            match command {
                Command::FetchApis => Ok(AppEvent::ApisLoaded(client.list_apis().await?.apis)),
                Command::FetchContents => {
                    let endpoint = endpoint.context("endpoint is missing")?;
                    let collection = client
                        .get_content_collection(&endpoint, limit, offset, &query)
                        .await?;
                    let (statuses, status_warning) = if collection.kind
                        == ContentCollectionKind::List
                    {
                        match client.list_content_metadata(&endpoint, limit, offset).await {
                            Ok(metadata) => {
                                let statuses: HashMap<_, _> = metadata
                                    .contents
                                    .into_iter()
                                    .map(|content| {
                                        (
                                            content.id,
                                            content_publication_state(&content.status),
                                        )
                                    })
                                    .collect();
                                let has_missing = collection.contents.iter().any(|value| {
                                    content_id(value)
                                        .map_or(true, |id| !statuses.contains_key(id))
                                });
                                let warning = has_missing.then(|| {
                                    "Content loaded; status metadata could not be matched for some items (query/filter/order may affect alignment).".to_string()
                                });
                                (statuses, warning)
                            }
                            Err(error) => (
                                HashMap::new(),
                                Some(format!(
                                    "Content loaded; status metadata unavailable: {error:#}"
                                )),
                            ),
                        }
                    } else {
                        (HashMap::new(), None)
                    };
                    let (create_template, content_field_order, schema_warning) = if needs_schema {
                        match client.get_api_schema(&endpoint).await {
                            Ok(schema) => schema_load_result(schema),
                            Err(error) => (
                                None,
                                Some(Vec::new()),
                                Some(format!(
                                    "Schema unavailable; cannot create content: {error:#}"
                                )),
                            ),
                        }
                    } else {
                        (None, None, None)
                    };
                    Ok(AppEvent::ContentsLoaded {
                        endpoint,
                        collection,
                        statuses,
                        status_warning,
                        create_template,
                        content_field_order,
                        schema_warning,
                    })
                }
                _ => bail!("invalid fetch command"),
            }
        }
        .await;

        let event = match result {
            Ok(event) => event,
            Err(error) => AppEvent::FetchFailed {
                endpoint: failure_endpoint,
                error: format!("{error:#}"),
            },
        };
        let _ = tx.send(event);
    });
}

fn schedule_mutation(app: &App, mutation: MutationRequest, tx: mpsc::UnboundedSender<AppEvent>) {
    let service_id = app.config.service_id.clone();
    let api_key = app.config.api_key.clone();
    let endpoint = app.endpoint.clone();

    tokio::spawn(async move {
        let event_endpoint = endpoint.clone().unwrap_or_default();
        let result: Result<AppEvent> = async {
            let service_id = service_id.context("service ID is missing")?;
            let api_key = api_key.context("API key is missing")?;
            let endpoint = endpoint.context("endpoint is missing")?;
            let client = MicroCmsClient::new(service_id, api_key)?;

            match mutation {
                MutationRequest::Create { value, status } => {
                    client.create_content(&endpoint, &value, status).await?;
                    Ok(AppEvent::MutationSucceeded {
                        endpoint: endpoint.clone(),
                        message: create_success_message(status),
                    })
                }
                MutationRequest::PutCreate {
                    content_id,
                    value,
                    status,
                } => {
                    client
                        .put_content(&endpoint, &content_id, &value, status)
                        .await?;
                    Ok(AppEvent::MutationSucceeded {
                        endpoint: endpoint.clone(),
                        message: put_create_success_message(status, &content_id),
                    })
                }
                MutationRequest::Update {
                    content_id,
                    value,
                    status,
                } => {
                    client
                        .update_content(&endpoint, &content_id, &value, status)
                        .await?;
                    Ok(AppEvent::MutationSucceeded {
                        endpoint: endpoint.clone(),
                        message: update_success_message(status),
                    })
                }
                MutationRequest::Delete { content_ids } => {
                    let count = content_ids.len();
                    for (index, content_id) in content_ids.iter().enumerate() {
                        client
                            .delete_content(&endpoint, content_id)
                            .await
                            .with_context(|| {
                                format!(
                                    "failed to delete content {}/{} ({content_id})",
                                    index + 1,
                                    count
                                )
                            })?;
                    }
                    Ok(AppEvent::MutationSucceeded {
                        endpoint: endpoint.clone(),
                        message: if count == 1 {
                            "Content deleted; page reloaded.".into()
                        } else {
                            format!("{count} contents deleted; page reloaded.")
                        },
                    })
                }
                MutationRequest::Status {
                    content_ids,
                    status,
                } => {
                    let count = content_ids.len();
                    for (index, content_id) in content_ids.iter().enumerate() {
                        client
                            .update_publication_status(&endpoint, content_id, status)
                            .await
                            .with_context(|| {
                                format!(
                                    "failed to update status {}/{} ({content_id})",
                                    index + 1,
                                    count
                                )
                            })?;
                    }
                    Ok(AppEvent::StatusSucceeded {
                        endpoint: endpoint.clone(),
                        message: match (status, count) {
                            (PublicationStatus::Publish, 1) => "Content published.".into(),
                            (PublicationStatus::Publish, count) => {
                                format!("{count} contents published.")
                            }
                            (PublicationStatus::Draft, 1) => "Content set to draft.".into(),
                            (PublicationStatus::Draft, count) => {
                                format!("{count} contents set to draft.")
                            }
                        },
                    })
                }
            }
        }
        .await;

        let event = match result {
            Ok(event) => event,
            Err(error) => AppEvent::MutationFailed {
                endpoint: event_endpoint,
                error: format!("{error:#}"),
            },
        };
        let _ = tx.send(event);
    });
}

fn create_success_message(status: ContentWriteStatus) -> String {
    match status {
        ContentWriteStatus::Default => "Content created; page reloaded.".into(),
        ContentWriteStatus::Draft => "Draft content created; page reloaded.".into(),
    }
}

fn put_create_success_message(status: ContentWriteStatus, content_id: &str) -> String {
    match status {
        ContentWriteStatus::Default => {
            format!("Content created with ID {content_id}; page reloaded.")
        }
        ContentWriteStatus::Draft => {
            format!("Draft content created with ID {content_id}; page reloaded.")
        }
    }
}

fn update_success_message(status: ContentWriteStatus) -> String {
    match status {
        ContentWriteStatus::Default => "Content updated; page reloaded.".into(),
        ContentWriteStatus::Draft => "Draft content updated; page reloaded.".into(),
    }
}

fn edit_json(terminal: &mut TuiTerminal, operation: &str, initial: &Value) -> Result<EditResult> {
    let path = temp_json_path(operation)?;
    let mut contents = serde_json::to_string_pretty(initial).context("failed to serialize JSON")?;
    contents.push('\n');
    fs::write(&path, contents)
        .with_context(|| format!("failed to write temporary file {}", path.display()))?;

    let suspend_result = restore_terminal(terminal);
    if let Err(error) = suspend_result {
        let _ = fs::remove_file(&path);
        return Err(error).context("failed to suspend TUI for editor");
    }

    let editor_result = run_editor(&path);
    let resume_result = resume_terminal(terminal);
    if let Err(error) = resume_result {
        let _ = fs::remove_file(&path);
        return Err(error).context("failed to restore TUI after editor");
    }
    if let Err(error) = editor_result {
        let _ = fs::remove_file(&path);
        return Err(error);
    }

    let read_result = fs::read_to_string(&path)
        .with_context(|| format!("failed to read edited JSON from {}", path.display()));
    let _ = fs::remove_file(&path);
    let edited = read_result?;
    edited_json_result(initial, &edited)
}

fn edited_json_result(initial: &Value, edited: &str) -> Result<EditResult> {
    let edited: Value =
        serde_json::from_str(edited).context("editor contents are not valid JSON")?;
    if &edited == initial {
        Ok(EditResult::Unchanged)
    } else {
        Ok(EditResult::Changed(edited))
    }
}

fn run_editor(path: &Path) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "vi".to_string());
    let mut parts = editor.split_whitespace();
    let program = parts.next().context("EDITOR does not contain a program")?;
    let status = ProcessCommand::new(program)
        .args(parts)
        .arg(path)
        .status()
        .with_context(|| format!("failed to launch editor '{program}'"))?;
    if !status.success() {
        bail!("editor exited with status {status}");
    }
    Ok(())
}

fn temp_json_path(operation: &str) -> Result<PathBuf> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_nanos();
    Ok(std::env::temp_dir().join(format!(
        "microcms-tui-{}-{operation}-{timestamp}.json",
        std::process::id()
    )))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn equal_json_with_different_formatting_is_unchanged() {
        let initial = json!({"title": "Post", "count": 1});
        let edited = "{\n  \"count\": 1,\n  \"title\": \"Post\"\n}\n";

        assert_eq!(
            edited_json_result(&initial, edited).unwrap(),
            EditResult::Unchanged
        );
    }

    #[test]
    fn changed_and_invalid_json_are_distinguished() {
        let initial = json!({"title": "Before"});
        assert_eq!(
            edited_json_result(&initial, r#"{"title":"After"}"#).unwrap(),
            EditResult::Changed(json!({"title": "After"}))
        );
        assert!(edited_json_result(&initial, "not json").is_err());
    }

    #[test]
    fn enter_selects_endpoint_or_toggles_content_preview() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::TogglePreviewFullscreen)
        );

        app.preview_fullscreen = true;
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::ClosePreviewFullscreen)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Esc), &app),
            Some(Action::ClosePreviewFullscreen)
        );

        app.preview_fullscreen = false;
        app.screen = Screen::EndpointPicker;
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::Select)
        );
    }

    #[test]
    fn fullscreen_preview_keymap_disables_context_changing_actions() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.preview_fullscreen = true;

        for code in [
            KeyCode::Char(' '),
            KeyCode::Char('c'),
            KeyCode::Char('C'),
            KeyCode::Char('u'),
            KeyCode::Char('U'),
            KeyCode::Char('/'),
            KeyCode::Char('f'),
            KeyCode::Char('o'),
            KeyCode::Char('x'),
            KeyCode::Char('b'),
            KeyCode::Char('r'),
        ] {
            assert_eq!(action_for_key(key(code), &app), None);
        }

        assert_eq!(
            action_for_key(key(KeyCode::Char('e')), &app),
            Some(Action::Edit)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('E')), &app),
            Some(Action::EditDraft)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('d')), &app),
            Some(Action::DeleteRequest)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('P')), &app),
            Some(Action::Publish)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('D')), &app),
            Some(Action::Draft)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('n')), &app),
            Some(Action::PreviewNextContent)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('p')), &app),
            Some(Action::PreviewPrevContent)
        );
    }

    #[test]
    fn ctrl_c_quits_before_input_delete_or_fullscreen_keymaps() {
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        let mut app = App::new(Config::default());

        assert_eq!(action_for_key(ctrl_c, &app), Some(Action::Quit));
        app.input_target = Some(crate::app::InputTarget::Search);
        assert_eq!(action_for_key(ctrl_c, &app), Some(Action::Quit));
        app.input_target = None;
        app.pending_confirmation = Some(PendingConfirmation::Delete {
            content_ids: vec!["content-id".into()],
        });
        assert_eq!(action_for_key(ctrl_c, &app), Some(Action::Quit));
        app.pending_confirmation = None;
        app.preview_fullscreen = true;
        assert_eq!(action_for_key(ctrl_c, &app), Some(Action::Quit));
        app.help_open = true;
        assert_eq!(action_for_key(ctrl_c, &app), Some(Action::Quit));
    }

    #[test]
    fn help_opens_in_browser_fullscreen_and_delete_confirmation() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        let help_key = key(KeyCode::Char('?'));

        assert_eq!(action_for_key(help_key, &app), Some(Action::ToggleHelp));
        app.preview_fullscreen = true;
        assert_eq!(action_for_key(help_key, &app), Some(Action::ToggleHelp));
        app.pending_confirmation = Some(PendingConfirmation::Delete {
            content_ids: vec!["content-id".into()],
        });
        assert_eq!(action_for_key(help_key, &app), Some(Action::ToggleHelp));
        assert!(app.pending_confirmation.is_some());
    }

    #[test]
    fn input_question_mark_is_text_instead_of_help() {
        let mut app = App::new(Config::default());
        app.input_target = Some(crate::app::InputTarget::Search);
        assert_eq!(
            action_for_key(key(KeyCode::Char('?')), &app),
            Some(Action::InputChar('?'))
        );
    }

    #[test]
    fn open_help_closes_only_with_help_enter_or_escape() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.preview_fullscreen = true;
        app.pending_confirmation = Some(PendingConfirmation::Delete {
            content_ids: vec!["content-id".into()],
        });
        app.help_open = true;

        assert_eq!(
            action_for_key(key(KeyCode::Char('?')), &app),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::CloseHelp)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Esc), &app),
            Some(Action::CloseHelp)
        );
        assert_eq!(action_for_key(key(KeyCode::Char('y')), &app), None);
        assert_eq!(action_for_key(key(KeyCode::Char('d')), &app), None);
        assert!(app.pending_confirmation.is_some());
    }

    #[test]
    fn pending_confirmation_blocks_underlying_actions_and_accepts_confirm_or_cancel() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.pending_confirmation = Some(PendingConfirmation::PublicationStatus {
            content_ids: vec!["content-id".into()],
            status: PublicationStatus::Publish,
        });

        assert_eq!(
            action_for_key(key(KeyCode::Char('y')), &app),
            Some(Action::ConfirmPending)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('n')), &app),
            Some(Action::CancelPending)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Esc), &app),
            Some(Action::CancelPending)
        );
        assert_eq!(action_for_key(key(KeyCode::Char('e')), &app), None);
        assert_eq!(action_for_key(key(KeyCode::Char('P')), &app), None);
    }

    #[test]
    fn default_writes_become_confirmations_and_draft_writes_remain_immediate() {
        let create = MutationRequest::Create {
            value: json!({"title": "Published"}),
            status: ContentWriteStatus::Default,
        };
        assert!(matches!(
            create.into_write_confirmation(),
            Ok(PendingConfirmation::Create { .. })
        ));

        let update = MutationRequest::Update {
            content_id: "content-id".into(),
            value: json!({"title": "Published"}),
            status: ContentWriteStatus::Default,
        };
        assert!(matches!(
            update.into_write_confirmation(),
            Ok(PendingConfirmation::Update { .. })
        ));

        let put_create = MutationRequest::PutCreate {
            content_id: "content-id".into(),
            value: json!({"title": "Published"}),
            status: ContentWriteStatus::Default,
        };
        assert!(matches!(
            put_create.into_write_confirmation(),
            Ok(PendingConfirmation::PutCreate { .. })
        ));

        let draft_create = MutationRequest::Create {
            value: json!({"title": "Draft"}),
            status: ContentWriteStatus::Draft,
        };
        assert!(matches!(
            draft_create.into_write_confirmation(),
            Err(MutationRequest::Create {
                status: ContentWriteStatus::Draft,
                ..
            })
        ));

        let draft_update = MutationRequest::Update {
            content_id: "content-id".into(),
            value: json!({"title": "Draft"}),
            status: ContentWriteStatus::Draft,
        };
        assert!(matches!(
            draft_update.into_write_confirmation(),
            Err(MutationRequest::Update {
                status: ContentWriteStatus::Draft,
                ..
            })
        ));

        let draft_put_create = MutationRequest::PutCreate {
            content_id: "content-id".into(),
            value: json!({"title": "Draft"}),
            status: ContentWriteStatus::Draft,
        };
        assert!(matches!(
            draft_put_create.into_write_confirmation(),
            Err(MutationRequest::PutCreate {
                status: ContentWriteStatus::Draft,
                ..
            })
        ));
    }

    #[test]
    fn plain_q_never_quits() {
        let mut app = App::new(Config::default());
        assert_eq!(action_for_key(key(KeyCode::Char('q')), &app), None);

        app.input_target = Some(crate::app::InputTarget::Search);
        assert_eq!(
            action_for_key(key(KeyCode::Char('q')), &app),
            Some(Action::InputChar('q'))
        );
        app.input_target = None;
        app.pending_confirmation = Some(PendingConfirmation::Delete {
            content_ids: vec!["content-id".into()],
        });
        assert_eq!(action_for_key(key(KeyCode::Char('q')), &app), None);
        app.pending_confirmation = None;
        app.preview_fullscreen = true;
        assert_eq!(action_for_key(key(KeyCode::Char('q')), &app), None);
    }

    #[test]
    fn normal_write_keys_distinguish_default_and_draft_actions() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;

        assert_eq!(
            action_for_key(key(KeyCode::Char('c')), &app),
            Some(Action::Create)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('C')), &app),
            Some(Action::CreateDraft)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('u')), &app),
            Some(Action::CreateWithId)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('U')), &app),
            Some(Action::CreateWithIdDraft)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('e')), &app),
            Some(Action::Edit)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('E')), &app),
            Some(Action::EditDraft)
        );
    }

    #[test]
    fn write_success_messages_distinguish_default_and_draft() {
        assert_eq!(
            create_success_message(ContentWriteStatus::Default),
            "Content created; page reloaded."
        );
        assert_eq!(
            create_success_message(ContentWriteStatus::Draft),
            "Draft content created; page reloaded."
        );
        assert_eq!(
            put_create_success_message(ContentWriteStatus::Default, "content-id"),
            "Content created with ID content-id; page reloaded."
        );
        assert_eq!(
            put_create_success_message(ContentWriteStatus::Draft, "content-id"),
            "Draft content created with ID content-id; page reloaded."
        );
        assert_eq!(
            update_success_message(ContentWriteStatus::Default),
            "Content updated; page reloaded."
        );
        assert_eq!(
            update_success_message(ContentWriteStatus::Draft),
            "Draft content updated; page reloaded."
        );
    }
}
