// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer Abdullah and Concat contributors

//! Live mode (#337): the API's calls on the project on screen, run here as
//! the window's own. An agent's edit is one step of the window's undo
//! history, the preview and timeline follow it, and the autosave writes
//! it, because it goes through the same [`Studio`] paths a click does.
//!
//! The API's server calls [`Window::run`] on its own thread; the call
//! crosses to the event loop with `slint::invoke_from_event_loop`, and the
//! server's thread waits for the answer. The window never waits on the
//! server, so the two cannot hold each other up.

use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use concat_api::{ApiError, ErrorCode, LiveCall, LiveReply, LiveWindow};

use super::*;

/// How long a call waits for the window: a modal dialog holds the event
/// loop, and an agent is better told so than left hanging.
const PATIENCE: Duration = Duration::from_secs(30);

/// The window, as the API's server reaches it.
pub(crate) struct Window;

impl LiveWindow for Window {
    fn run(&self, path: &str, call: LiveCall) -> Result<LiveReply, ApiError> {
        let (answer, wait) = mpsc::channel();
        let path = path.to_owned();
        slint::invoke_from_event_loop(move || {
            crate::host::Shell::with(|shell, app| {
                let reply = shell.studio.borrow_mut().live(&path, call);
                shell.studio.borrow().publish(&app, &shell.models);
                let _ = answer.send(reply);
            });
        })
        .map_err(|_| closed())?;
        wait.recv_timeout(PATIENCE).map_err(|error| match error {
            RecvTimeoutError::Timeout => {
                ApiError::failed("the window did not answer in time: a dialog may be open in it")
            }
            RecvTimeoutError::Disconnected => closed(),
        })?
    }
}

fn closed() -> ApiError {
    ApiError::failed("the window has closed")
}

/// Whether two spellings name one folder.
fn same_folder(left: &str, right: &str) -> bool {
    let canonical = |path: &str| std::fs::canonicalize(path).unwrap_or_else(|_| path.into());
    canonical(left) == canonical(right)
}

impl Studio {
    /// Runs one of the API's calls on the open project, which must be the
    /// one at `path`: the window may have moved on to another since the
    /// call was sent.
    fn live(&mut self, path: &str, call: LiveCall) -> Result<LiveReply, ApiError> {
        let Some(session) = self
            .session
            .as_ref()
            .filter(|session| same_folder(session.path(), path))
        else {
            return Err(ApiError::new(
                ErrorCode::NotOpen,
                format!("{path} is no longer open in the window"),
            ));
        };
        let edit = |studio: &mut Studio, command: Command| match studio.try_apply(command) {
            Some(Ok(view)) => Ok(LiveReply::View(view)),
            Some(Err(error)) => Err(ApiError::new(ErrorCode::Refused, error)),
            None => Err(ApiError::new(ErrorCode::NotOpen, "no project is open")),
        };
        match call {
            LiveCall::View => Ok(LiveReply::View(session.view())),
            LiveCall::Document => Ok(LiveReply::Document(session.document())),
            LiveCall::Detach => Ok(LiveReply::Detached(Box::new(session.detached()))),
            LiveCall::SetVideo(video) => {
                let timeline_id = session.project().active_timeline_id.clone();
                edit(self, Command::SetTimelineVideo { timeline_id, video })
            }
            LiveCall::Apply(command) => edit(self, *command),
            LiveCall::Undo => {
                self.undo();
                Ok(LiveReply::View(self.view_now()))
            }
            LiveCall::Redo => {
                self.redo();
                Ok(LiveReply::View(self.view_now()))
            }
            // The window's save, written on its worker in its order: the
            // reply says it is on its way, as a click on Save would.
            LiveCall::Save => {
                self.save(false);
                Ok(LiveReply::Done)
            }
        }
    }

    /// The open project as it stands; only called with one open.
    fn view_now(&self) -> concat_host::session::EditorView {
        self.session
            .as_ref()
            .map(Session::view)
            .expect("a project is open")
    }
}
