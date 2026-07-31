use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    process::Command as ProcessCommand,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, Context, Result};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    Terminal,
};
use serde_json::Value;
use tokio::sync::mpsc;

use crate::{
    app::{
        content_field_order_from_api_schema, content_id, content_publication_state,
        create_template_from_api_schema, sanitized_payload, Action, App, AppEvent, Command,
        LoadState, PendingConfirmation, Screen, TextEditAction,
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
    Reservation {
        content_id: String,
        publish_time: Option<String>,
        stop_time: Option<String>,
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
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
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
            let _ = execute!(stdout, DisableMouseCapture, LeaveAlternateScreen);
            let _ = disable_raw_mode();
            Err(error).context("failed to initialize terminal")
        }
    }
}

fn restore_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    let raw_result = disable_raw_mode().context("failed to disable terminal raw mode");
    let screen_result = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    )
    .context("failed to leave alternate screen");
    let cursor_result = terminal.show_cursor().context("failed to show cursor");
    raw_result.and(screen_result).and(cursor_result)
}

fn resume_terminal(terminal: &mut TuiTerminal) -> Result<()> {
    enable_raw_mode().context("failed to re-enable terminal raw mode")?;
    if let Err(error) = execute!(
        terminal.backend_mut(),
        EnterAlternateScreen,
        EnableMouseCapture
    ) {
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
            let terminal_event = event::read().context("failed to read terminal event")?;
            let action = match terminal_event {
                Event::Key(key)
                    if matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    action_for_key(key, &app)
                }
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    action_for_mouse(mouse, &app, Rect::new(0, 0, size.width, size.height))
                }
                _ => None,
            };
            if let Some(action) = action {
                let command = app.apply_action(action);
                handle_command(terminal, &mut app, command, tx.clone());
            }
        }
    }
    Ok(())
}

fn action_for_mouse(mouse: MouseEvent, app: &App, terminal_area: Rect) -> Option<Action> {
    if app.help_open
        || app.input_target.is_some()
        || app.reservation_input.is_some()
        || app.pending_confirmation.is_some()
        || app.version_comparison.is_some()
    {
        return None;
    }

    if app.query_selector.is_some() {
        return match mouse.kind {
            MouseEventKind::ScrollDown => Some(Action::QuerySelectorMoveDown),
            MouseEventKind::ScrollUp => Some(Action::QuerySelectorMoveUp),
            MouseEventKind::Down(MouseButton::Left) => {
                query_selector_index_at(app, terminal_area, mouse.column, mouse.row)
                    .map(Action::QuerySelectorChoose)
            }
            _ => None,
        };
    }

    if app.screen == Screen::ContentBrowser
        && (app.preview_fullscreen || app.content_kind == ContentCollectionKind::Object)
    {
        return match mouse.kind {
            MouseEventKind::ScrollDown => Some(Action::PreviewScrollDown),
            MouseEventKind::ScrollUp => Some(Action::PreviewScrollUp),
            _ => None,
        };
    }

    let main = main_area(terminal_area);
    let columns = match app.screen {
        Screen::EndpointPicker => horizontal_columns(main, 60),
        Screen::ContentBrowser => horizontal_columns(main, 40),
    };
    let list_area = columns[0];
    if app.screen == Screen::ContentBrowser && rect_contains(columns[1], mouse.column, mouse.row) {
        return match mouse.kind {
            MouseEventKind::ScrollDown => Some(Action::PreviewScrollDown),
            MouseEventKind::ScrollUp => Some(Action::PreviewScrollUp),
            _ => None,
        };
    }
    if !rect_contains(list_area, mouse.column, mouse.row) {
        return None;
    }
    match mouse.kind {
        MouseEventKind::ScrollDown => Some(Action::MoveDown),
        MouseEventKind::ScrollUp => Some(Action::MoveUp),
        MouseEventKind::Down(MouseButton::Left) => {
            let (selected, len) = match app.screen {
                Screen::EndpointPicker => (app.api_selected, app.apis.len()),
                Screen::ContentBrowser => (app.content_selected, app.items.len()),
            };
            list_index_at(list_area, selected, len, mouse.column, mouse.row).map(|index| {
                if app.screen == Screen::EndpointPicker {
                    Action::SelectApiAt(index)
                } else {
                    Action::SelectContentAt(index)
                }
            })
        }
        _ => None,
    }
}

fn main_area(area: Rect) -> Rect {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area)[1]
}

fn horizontal_columns(area: Rect, left_percent: u16) -> [Rect; 2] {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_percent),
            Constraint::Percentage(100 - left_percent),
        ])
        .split(area);
    [columns[0], columns[1]]
}

fn rect_contains(area: Rect, column: u16, row: u16) -> bool {
    column >= area.x
        && column < area.x.saturating_add(area.width)
        && row >= area.y
        && row < area.y.saturating_add(area.height)
}

fn list_index_at(area: Rect, selected: usize, len: usize, column: u16, row: u16) -> Option<usize> {
    let inner = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(2),
        area.height.saturating_sub(2),
    );
    if !rect_contains(inner, column, row) || len == 0 {
        return None;
    }
    let visible_rows = inner.height as usize;
    let first = selected
        .saturating_add(1)
        .saturating_sub(visible_rows.max(1));
    let index = first + usize::from(row - inner.y);
    (index < len).then_some(index)
}

fn query_selector_index_at(app: &App, area: Rect, column: u16, row: u16) -> Option<usize> {
    let (cursor, len) = match app.query_selector.as_ref()? {
        crate::app::QuerySelector::Fields { cursor, .. } => {
            (*cursor, app.content_field_order.len())
        }
        crate::app::QuerySelector::Depth { cursor } => (*cursor, 5),
        crate::app::QuerySelector::RichEditorFormat { cursor } => (*cursor, 3),
    };
    let height = (len as u16 + 2).clamp(5, 20);
    let modal = centered_modal_rect(area, 64, height, 72);
    let visible_rows = modal.height.saturating_sub(2) as usize;
    let first = cursor.saturating_add(1).saturating_sub(visible_rows.max(1));
    let inner = Rect::new(
        modal.x.saturating_add(1),
        modal.y.saturating_add(1),
        modal.width.saturating_sub(2),
        modal.height.saturating_sub(2),
    );
    if !rect_contains(inner, column, row) {
        return None;
    }
    let index = first + usize::from(row - inner.y);
    (index < len).then_some(index)
}

fn centered_modal_rect(area: Rect, percent_width: u16, height: u16, max_width: u16) -> Rect {
    let width = area
        .width
        .saturating_mul(percent_width)
        .checked_div(100)
        .unwrap_or(area.width)
        .min(max_width)
        .max(1)
        .min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
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
            KeyCode::Enter => Some(Action::InputApply),
            KeyCode::Esc => Some(Action::InputCancel),
            _ => text_edit_action(key)
                .map(Action::InputEdit)
                .or_else(|| printable_character(key).map(Action::InputChar)),
        };
    }
    if app.query_selector.is_some() {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::QuerySelectorMoveDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::QuerySelectorMoveUp),
            KeyCode::Char(' ') => Some(Action::QuerySelectorToggle),
            KeyCode::Enter => Some(Action::QuerySelectorApply),
            KeyCode::Esc => Some(Action::QuerySelectorCancel),
            _ => None,
        };
    }
    if app.reservation_input.is_some() {
        return match code {
            KeyCode::Char('?') if key.modifiers.is_empty() => Some(Action::ToggleHelp),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                Some(Action::ReservationNextField)
            }
            KeyCode::Enter => Some(Action::ReservationApply),
            KeyCode::F(8) => Some(Action::ReservationClear),
            KeyCode::Esc => Some(Action::ReservationCancel),
            _ => text_edit_action(key)
                .map(Action::ReservationEdit)
                .or_else(|| printable_character(key).map(Action::ReservationInputChar)),
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
    if app.version_comparison.is_some() {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Esc | KeyCode::Enter => Some(Action::CloseVersionComparison),
            KeyCode::Char('1') => Some(Action::VersionPublished),
            KeyCode::Char('2') => Some(Action::VersionDraft),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::VersionScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::VersionScrollUp),
            _ => None,
        };
    }
    if app.screen == Screen::ContentBrowser && app.content_kind == ContentCollectionKind::Object {
        return match code {
            KeyCode::Char('?') => Some(Action::ToggleHelp),
            KeyCode::Esc | KeyCode::Char('b') => Some(Action::Back),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::PreviewScrollDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::PreviewScrollUp),
            KeyCode::Char('g') => Some(Action::PreviewScrollTop),
            KeyCode::Char('G') => Some(Action::PreviewScrollBottom),
            KeyCode::Char('r') => Some(Action::Reload),
            KeyCode::Char('/') => Some(Action::EditSearch),
            KeyCode::Char('f') => Some(Action::EditFilters),
            KeyCode::Char('o') => Some(Action::EditOrders),
            KeyCode::Char('l') => Some(Action::EditFields),
            KeyCode::Char('z') => Some(Action::EditDepth),
            KeyCode::Char('i') => Some(Action::EditIds),
            KeyCode::Char('K') => Some(Action::EditDraftKey),
            KeyCode::Char('m') => Some(Action::EditRichEditorFormat),
            KeyCode::Char('x') => Some(Action::ClearQuery),
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
            KeyCode::Char('s') => Some(Action::EditReservation),
            KeyCode::Char('v') => Some(Action::CompareVersions),
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
        KeyCode::Char('l') if app.screen == Screen::ContentBrowser => Some(Action::EditFields),
        KeyCode::Char('z') if app.screen == Screen::ContentBrowser => Some(Action::EditDepth),
        KeyCode::Char('i') if app.screen == Screen::ContentBrowser => Some(Action::EditIds),
        KeyCode::Char('K') if app.screen == Screen::ContentBrowser => Some(Action::EditDraftKey),
        KeyCode::Char('m') if app.screen == Screen::ContentBrowser => {
            Some(Action::EditRichEditorFormat)
        }
        KeyCode::Char('x') if app.screen == Screen::ContentBrowser => Some(Action::ClearQuery),
        KeyCode::Char('P') if app.screen == Screen::ContentBrowser => Some(Action::Publish),
        KeyCode::Char('D') if app.screen == Screen::ContentBrowser => Some(Action::Draft),
        KeyCode::Char('s') if app.screen == Screen::ContentBrowser => Some(Action::EditReservation),
        KeyCode::Char('v') if app.screen == Screen::ContentBrowser => Some(Action::CompareVersions),
        KeyCode::Char('n') | KeyCode::PageDown if app.screen == Screen::ContentBrowser => {
            Some(Action::NextPage)
        }
        KeyCode::Char('p') | KeyCode::PageUp if app.screen == Screen::ContentBrowser => {
            Some(Action::PrevPage)
        }
        _ => None,
    }
}

fn printable_character(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(character)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            Some(character)
        }
        _ => None,
    }
}

fn text_edit_action(key: KeyEvent) -> Option<TextEditAction> {
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Backspace if alt => Some(TextEditAction::DeletePrevWord),
        KeyCode::Backspace => Some(TextEditAction::Backspace),
        KeyCode::Delete => Some(TextEditAction::Delete),
        KeyCode::Home => Some(TextEditAction::MoveStart),
        KeyCode::End => Some(TextEditAction::MoveEnd),
        KeyCode::Left if control || alt => Some(TextEditAction::MoveWordLeft),
        KeyCode::Right if control || alt => Some(TextEditAction::MoveWordRight),
        KeyCode::Left => Some(TextEditAction::MoveLeft),
        KeyCode::Right => Some(TextEditAction::MoveRight),
        KeyCode::Char('a') if control => Some(TextEditAction::MoveStart),
        KeyCode::Char('e') if control => Some(TextEditAction::MoveEnd),
        KeyCode::Char('b') if control => Some(TextEditAction::MoveLeft),
        KeyCode::Char('f') if control => Some(TextEditAction::MoveRight),
        KeyCode::Char('b') if alt => Some(TextEditAction::MoveWordLeft),
        KeyCode::Char('f') if alt => Some(TextEditAction::MoveWordRight),
        KeyCode::Char('h') if control => Some(TextEditAction::Backspace),
        KeyCode::Char('d') if control => Some(TextEditAction::Delete),
        KeyCode::Char('u') if control => Some(TextEditAction::DeleteToStart),
        KeyCode::Char('k') if control => Some(TextEditAction::DeleteToEnd),
        KeyCode::Char('w') if control => Some(TextEditAction::DeletePrevWord),
        KeyCode::Char('d') if alt => Some(TextEditAction::DeleteNextWord),
        KeyCode::Char('t') if control => Some(TextEditAction::Transpose),
        KeyCode::Char('y') if control => Some(TextEditAction::Yank),
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
        fetch @ (Command::FetchApis
        | Command::FetchContents
        | Command::FetchVersions { .. }
        | Command::FetchReservation { .. }) => schedule_fetch(app, fetch, tx),
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
                MutationRequest::Delete { .. }
                | MutationRequest::Status { .. }
                | MutationRequest::Reservation { .. } => {
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
        PendingConfirmation::Reservation {
            content_id,
            publish_time,
            stop_time,
        } => {
            app.message = Some(if publish_time.is_none() && stop_time.is_none() {
                "Removing publication reservation...".into()
            } else {
                "Updating publication reservation...".into()
            });
            MutationRequest::Reservation {
                content_id,
                publish_time,
                stop_time,
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
    let expected_content_kind = app.content_kind_confirmed.then_some(app.content_kind);
    let query = ContentQuery {
        q: app.search_query.clone(),
        filters: app.filters.clone(),
        orders: app.orders.clone(),
        fields: fields_with_content_id(app.fields.as_deref()),
        depth: app.depth,
        ids: app.ids.clone(),
        draft_key: app.draft_key.clone(),
        rich_editor_format: app.rich_editor_format.clone(),
    };
    let needs_schema = should_fetch_schema(app, endpoint.as_deref());
    let is_version_fetch = matches!(&command, Command::FetchVersions { .. });
    let is_reservation_fetch = matches!(&command, Command::FetchReservation { .. });
    let auxiliary_content_id = match &command {
        Command::FetchVersions { content_id } | Command::FetchReservation { content_id } => {
            content_id.clone()
        }
        _ => String::new(),
    };
    let failure_endpoint = match &command {
        Command::FetchContents
        | Command::FetchVersions { .. }
        | Command::FetchReservation { .. } => endpoint.clone(),
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
                        .get_content_collection(
                            &endpoint,
                            limit,
                            offset,
                            &query,
                            expected_content_kind,
                        )
                        .await?;
                    let (statuses, draft_keys, reservations, status_warning) = if collection.kind
                        == ContentCollectionKind::List
                    {
                        match client.list_content_metadata(&endpoint, limit, offset).await {
                            Ok(metadata) => {
                                let statuses: HashMap<_, _> = metadata
                                    .contents
                                    .iter()
                                    .map(|content| {
                                        (
                                            content.id.clone(),
                                            content_publication_state(&content.status),
                                        )
                                    })
                                    .collect();
                                let draft_keys = metadata
                                    .contents
                                    .iter()
                                    .filter_map(|content| {
                                        content
                                            .draft_key
                                            .as_ref()
                                            .filter(|key| !key.is_empty())
                                            .map(|key| (content.id.clone(), key.clone()))
                                    })
                                    .collect();
                                let reservations = metadata
                                    .contents
                                    .iter()
                                    .filter_map(|content| {
                                        content
                                            .reservation_time
                                            .clone()
                                            .map(|value| (content.id.clone(), value))
                                    })
                                    .collect();
                                let has_missing = collection.contents.iter().any(|value| {
                                    content_id(value)
                                        .map_or(true, |id| !statuses.contains_key(id))
                                });
                                let warning = has_missing.then(|| {
                                    "Content loaded; status metadata could not be matched for some items (query/filter/order may affect alignment).".to_string()
                                });
                                (statuses, draft_keys, reservations, warning)
                            }
                            Err(error) => (
                                HashMap::new(),
                                HashMap::new(),
                                HashMap::new(),
                                Some(format!(
                                    "Content loaded; status metadata unavailable: {error:#}"
                                )),
                            ),
                        }
                    } else {
                        (HashMap::new(), HashMap::new(), HashMap::new(), None)
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
                        draft_keys,
                        reservations,
                    })
                }
                Command::FetchVersions { content_id } => {
                    let endpoint = endpoint.context("endpoint is missing")?;
                    let metadata = client.get_content_metadata(&endpoint, &content_id).await?;
                    let draft_key = metadata.draft_key.filter(|value| !value.is_empty()).context(
                        "Selected content has no draftKey; no draft version is available.",
                    )?;
                    let base_query = ContentQuery {
                        fields: query.fields.clone(),
                        depth: query.depth,
                        rich_editor_format: query.rich_editor_format.clone(),
                        ..ContentQuery::default()
                    };
                    let published = client
                        .get_content_version(&endpoint, &content_id, &base_query)
                        .await?;
                    let draft_query = ContentQuery {
                        draft_key: Some(draft_key),
                        ..base_query
                    };
                    let draft = client
                        .get_content_version(&endpoint, &content_id, &draft_query)
                        .await?;
                    Ok(AppEvent::VersionsLoaded {
                        endpoint,
                        content_id,
                        published,
                        draft,
                    })
                }
                Command::FetchReservation { content_id } => {
                    let endpoint = endpoint.context("endpoint is missing")?;
                    let metadata = client.get_content_metadata(&endpoint, &content_id).await?;
                    Ok(AppEvent::ReservationLoaded {
                        endpoint,
                        content_id,
                        reservation: metadata.reservation_time,
                        publication_state: content_publication_state(&metadata.status),
                    })
                }
                _ => bail!("invalid fetch command"),
            }
        }
        .await;

        let event = match result {
            Ok(event) => event,
            Err(error) if is_version_fetch => AppEvent::VersionsFailed {
                endpoint: failure_endpoint.unwrap_or_default(),
                content_id: auxiliary_content_id,
                error: format!("{error:#}"),
            },
            Err(error) if is_reservation_fetch => AppEvent::ReservationFailed {
                endpoint: failure_endpoint.unwrap_or_default(),
                content_id: auxiliary_content_id,
                error: format!("{error:#}"),
            },
            Err(error) => AppEvent::FetchFailed {
                endpoint: failure_endpoint,
                error: format!("{error:#}"),
            },
        };
        let _ = tx.send(event);
    });
}

fn should_fetch_schema(app: &App, endpoint: Option<&str>) -> bool {
    endpoint.is_some_and(|endpoint| !app.schema_cache.contains_key(endpoint))
}

fn fields_with_content_id(fields: Option<&str>) -> Option<String> {
    let fields = fields?.trim();
    if fields.is_empty() {
        return None;
    }
    let mut values: Vec<&str> = fields
        .split(',')
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .collect();
    if !values.contains(&"id") {
        values.push("id");
    }
    Some(values.join(","))
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
                MutationRequest::Reservation {
                    content_id,
                    publish_time,
                    stop_time,
                } => {
                    client
                        .update_reservation(
                            &endpoint,
                            &content_id,
                            publish_time.as_deref(),
                            stop_time.as_deref(),
                        )
                        .await?;
                    Ok(AppEvent::MutationSucceeded {
                        endpoint: endpoint.clone(),
                        message: if publish_time.is_none() && stop_time.is_none() {
                            "Publication reservation removed; page reloaded.".into()
                        } else {
                            "Publication reservation updated; page reloaded.".into()
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

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn mouse_click_selects_content_row_and_wheel_moves_lists() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "one"}), json!({"id": "two"})];
        let area = Rect::new(0, 0, 80, 24);

        assert_eq!(
            action_for_mouse(
                mouse(MouseEventKind::Down(MouseButton::Left), 2, 3),
                &app,
                area
            ),
            Some(Action::SelectContentAt(1))
        );
        assert_eq!(
            action_for_mouse(mouse(MouseEventKind::ScrollDown, 2, 3), &app, area),
            Some(Action::MoveDown)
        );
    }

    #[test]
    fn mouse_wheel_scrolls_fullscreen_preview() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.preview_fullscreen = true;

        assert_eq!(
            action_for_mouse(
                mouse(MouseEventKind::ScrollDown, 40, 12),
                &app,
                Rect::new(0, 0, 80, 24)
            ),
            Some(Action::PreviewScrollDown)
        );
    }

    #[test]
    fn mouse_wheel_scrolls_normal_json_preview_pane() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.state = LoadState::ContentsLoaded;
        app.items = vec![json!({"id": "one"})];

        assert_eq!(
            action_for_mouse(
                mouse(MouseEventKind::ScrollDown, 60, 12),
                &app,
                Rect::new(0, 0, 80, 24)
            ),
            Some(Action::PreviewScrollDown)
        );
    }

    #[test]
    fn mouse_click_chooses_query_selector_row() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.content_field_order = vec!["title".into(), "body".into()];
        app.query_selector = Some(crate::app::QuerySelector::Fields {
            cursor: 0,
            selected: Default::default(),
        });
        let area = Rect::new(0, 0, 80, 24);
        let modal = centered_modal_rect(area, 64, 5, 72);

        assert_eq!(
            action_for_mouse(
                mouse(
                    MouseEventKind::Down(MouseButton::Left),
                    modal.x + 2,
                    modal.y + 2
                ),
                &app,
                area
            ),
            Some(Action::QuerySelectorChoose(1))
        );
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
    fn object_api_uses_direct_preview_keymap_without_fullscreen_toggle() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.content_kind = ContentCollectionKind::Object;

        assert_eq!(action_for_key(key(KeyCode::Enter), &app), None);
        assert_eq!(
            action_for_key(key(KeyCode::Char('j')), &app),
            Some(Action::PreviewScrollDown)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('G')), &app),
            Some(Action::PreviewScrollBottom)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('r')), &app),
            Some(Action::Reload)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('l')), &app),
            Some(Action::EditFields)
        );
        for code in [
            KeyCode::Char(' '),
            KeyCode::Char('c'),
            KeyCode::Char('e'),
            KeyCode::Char('d'),
            KeyCode::Char('P'),
            KeyCode::Char('n'),
        ] {
            assert_eq!(action_for_key(key(code), &app), None, "{code:?}");
        }
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
    fn reservation_comparison_and_extended_query_keys_follow_modal_priority() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        assert_eq!(
            action_for_key(key(KeyCode::Char('s')), &app),
            Some(Action::EditReservation)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('v')), &app),
            Some(Action::CompareVersions)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('l')), &app),
            Some(Action::EditFields)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('z')), &app),
            Some(Action::EditDepth)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('i')), &app),
            Some(Action::EditIds)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('K')), &app),
            Some(Action::EditDraftKey)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('m')), &app),
            Some(Action::EditRichEditorFormat)
        );

        app.reservation_input = Some(crate::app::ReservationInput {
            content_id: "id".into(),
            publish_time: String::new(),
            stop_time: String::new(),
            publish_cursor: 0,
            stop_cursor: 0,
            active_field: crate::app::ReservationField::PublishTime,
            publication_state: crate::app::ContentPublicationState::Draft,
        });
        assert_eq!(
            action_for_key(key(KeyCode::Tab), &app),
            Some(Action::ReservationNextField)
        );
        assert_eq!(
            action_for_key(key(KeyCode::F(8)), &app),
            Some(Action::ReservationClear)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Delete), &app),
            Some(Action::ReservationEdit(TextEditAction::Delete))
        );
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::ReservationApply)
        );
    }

    #[test]
    fn input_keymap_supports_terminal_line_editing_shortcuts() {
        let mut app = App::new(Config::default());
        app.input_target = Some(crate::app::InputTarget::Search);

        for (code, modifiers, edit) in [
            (
                KeyCode::Char('a'),
                KeyModifiers::CONTROL,
                TextEditAction::MoveStart,
            ),
            (
                KeyCode::Char('e'),
                KeyModifiers::CONTROL,
                TextEditAction::MoveEnd,
            ),
            (
                KeyCode::Char('b'),
                KeyModifiers::CONTROL,
                TextEditAction::MoveLeft,
            ),
            (
                KeyCode::Char('f'),
                KeyModifiers::CONTROL,
                TextEditAction::MoveRight,
            ),
            (
                KeyCode::Char('u'),
                KeyModifiers::CONTROL,
                TextEditAction::DeleteToStart,
            ),
            (
                KeyCode::Char('k'),
                KeyModifiers::CONTROL,
                TextEditAction::DeleteToEnd,
            ),
            (
                KeyCode::Char('w'),
                KeyModifiers::CONTROL,
                TextEditAction::DeletePrevWord,
            ),
            (
                KeyCode::Char('d'),
                KeyModifiers::ALT,
                TextEditAction::DeleteNextWord,
            ),
            (
                KeyCode::Char('y'),
                KeyModifiers::CONTROL,
                TextEditAction::Yank,
            ),
        ] {
            assert_eq!(
                action_for_key(KeyEvent::new(code, modifiers), &app),
                Some(Action::InputEdit(edit))
            );
        }

        assert_eq!(
            action_for_key(key(KeyCode::Char('q')), &app),
            Some(Action::InputChar('q'))
        );
    }

    #[test]
    fn ids_input_uses_the_standard_single_line_apply_key() {
        let mut app = App::new(Config::default());
        app.input_target = Some(crate::app::InputTarget::Ids);

        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::InputApply)
        );
    }

    #[test]
    fn fields_query_keeps_content_id_for_status_metadata_matching() {
        assert_eq!(fields_with_content_id(None), None);
        assert_eq!(
            fields_with_content_id(Some("title,body")),
            Some("title,body,id".into())
        );
        assert_eq!(
            fields_with_content_id(Some("title,id")),
            Some("title,id".into())
        );
    }

    #[test]
    fn query_selector_modal_blocks_browser_keys_and_handles_selection() {
        let mut app = App::new(Config::default());
        app.screen = Screen::ContentBrowser;
        app.query_selector = Some(crate::app::QuerySelector::Fields {
            cursor: 0,
            selected: Default::default(),
        });

        assert_eq!(
            action_for_key(key(KeyCode::Char(' ')), &app),
            Some(Action::QuerySelectorToggle)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Char('j')), &app),
            Some(Action::QuerySelectorMoveDown)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Enter), &app),
            Some(Action::QuerySelectorApply)
        );
        assert_eq!(
            action_for_key(key(KeyCode::Esc), &app),
            Some(Action::QuerySelectorCancel)
        );
        assert_eq!(action_for_key(key(KeyCode::Char('c')), &app), None);
    }

    #[test]
    fn schema_fetch_is_skipped_for_cached_endpoint() {
        let mut app = App::new(Config::default());
        assert!(should_fetch_schema(&app, Some("blogs")));
        app.schema_cache
            .insert("blogs".into(), crate::app::CachedSchema::default());
        assert!(!should_fetch_schema(&app, Some("blogs")));
        assert!(!should_fetch_schema(&app, None));
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
