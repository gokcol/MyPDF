//! Password-protected PDFs: asking for the password when one is opened,
//! protecting a copy with a password, and saving a copy without it.
use super::*;
use mypdf_milestone0::security;
use zeroize::Zeroizing;

/// A protected document waiting for its password.
pub(super) struct Locked {
    encrypted: Arc<[u8]>,
    typed: Zeroizing<String>,
    error: Option<String>,
    rx: Option<mpsc::Receiver<Result<Vec<u8>, String>>>,
}
impl Locked {
    pub fn new(encrypted: Arc<[u8]>) -> Self {
        Self {
            encrypted,
            typed: Zeroizing::new(String::new()),
            error: None,
            rx: None,
        }
    }
}
/// The "Protect with Password" dialog.
#[derive(Default)]
pub(super) struct ProtectDraft {
    password: Zeroizing<String>,
    confirm: Zeroizing<String>,
    error: Option<String>,
}

impl Viewer {
    /// Ask for the active document's password, and open it once it is right.
    pub(super) fn password_prompt(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let name = tab.name();
        let Some(locked) = &mut tab.locked else {
            return;
        };
        if let Some(rx) = &locked.rx
            && let Ok(result) = rx.try_recv()
        {
            locked.rx = None;
            match result {
                Ok(bytes) => {
                    let password = std::mem::take(&mut locked.typed);
                    tab.locked = None;
                    tab.password = Some(password);
                    let wake = ctx.clone();
                    tab.worker = Worker::from_bytes(bytes.into(), move || wake.request_repaint());
                    self.status = Some((
                        "Opened with its password • Copies you save keep the password".into(),
                        false,
                    ));
                    return;
                }
                Err(e) => {
                    locked.error = Some(e);
                    locked.typed = Zeroizing::new(String::new());
                }
            }
        }
        let working = locked.rx.is_some();
        let mut open = false;
        let mut cancel = false;
        let modal = egui::Modal::new(egui::Id::new(("password_prompt", tab.id)))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(320.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(
                        ui.painter(),
                        r,
                        Icon::Shield,
                        ui::Fill::Brand,
                        Color32::WHITE,
                    );
                    ui.add_space(12.);
                    ui::title(ui, "This PDF is protected", 16.);
                    ui.label(
                        RichText::new(format!("Enter the password to open “{name}”."))
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                let r = ui.add_enabled(
                    !working,
                    egui::TextEdit::singleline(&mut *locked.typed)
                        .margin(ui::FIELD_MARGIN)
                        .password(true)
                        .desired_width(f32::INFINITY)
                        .hint_text("Password")
                        .char_limit(127),
                );
                if !working && !r.has_focus() {
                    r.request_focus();
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    open = true;
                }
                if let Some(e) = &locked.error {
                    ui.label(RichText::new(e).size(12.).color(theme::danger()));
                }
                if working {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Opening…").color(theme::muted()));
                    });
                }
                ui.add_space(12.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel =
                            ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        open |= ui::dialog_button(ui, "Open", ui::DialogTone::Primary).clicked();
                    });
                });
            });
        if modal.should_close() && !working {
            cancel = true;
        }
        if open && !working && !locked.typed.is_empty() {
            let (tx, rx) = mpsc::channel();
            let encrypted = locked.encrypted.clone();
            let password = locked.typed.clone();
            let wake = ctx.clone();
            std::thread::spawn(move || {
                let result = security::unlock(&encrypted, &password).map_err(|e| format!("{e:#}"));
                let _ = tx.send(result);
                wake.request_repaint();
            });
            locked.rx = Some(rx);
            locked.error = None;
        }
        if cancel {
            self.close(self.active);
        }
    }

    pub(super) fn request_protect(&mut self) {
        let Some(tab) = self.tabs.get(self.active).filter(|t| t.session.is_some()) else {
            return;
        };
        if tab.signed || tab.signed_original.is_some() {
            self.status = Some((
                "A signed PDF cannot be protected here: that would rewrite it and break its signatures.".into(),
                true,
            ));
            return;
        }
        self.protect = Some(ProtectDraft::default());
    }
    pub(super) fn request_unprotect(&mut self) {
        if self
            .tabs
            .get(self.active)
            .is_some_and(|t| t.password.is_some())
        {
            self.unprotect = true;
        }
    }

    pub(super) fn protect_dialogs(&mut self, ctx: &egui::Context) {
        if self.unprotect {
            match ui::confirmation(
                ctx,
                "remove_password",
                "Save a copy without the password?",
                "The copy opens without a password. The file you opened keeps its password.",
                &[
                    ("Save a Copy…", ui::DialogTone::Primary),
                    ("Cancel", ui::DialogTone::Secondary),
                ],
                1,
            ) {
                Some(0) => {
                    self.unprotect = false;
                    self.save_protected(ctx, None, "unprotected");
                }
                Some(_) => self.unprotect = false,
                None => {}
            }
        }
        let Some(draft) = &mut self.protect else {
            return;
        };
        let mut save = false;
        let mut cancel = false;
        let mut focus = false;
        let modal = egui::Modal::new(egui::Id::new("protect_password"))
            .backdrop_color(Color32::from_black_alpha(64))
            .frame(ui::modal_frame())
            .show(ctx, |ui| {
                ui.set_width(340.);
                ui.vertical_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(54., 54.), egui::Sense::hover());
                    ui::badge(ui.painter(), r, Icon::Shield, ui::Fill::Brand, Color32::WHITE);
                    ui.add_space(12.);
                    ui::title(ui, "Protect with a password", 16.);
                    ui.label(
                        RichText::new("The copy you save opens only with this password.")
                            .size(13.)
                            .color(theme::muted()),
                    );
                });
                ui.add_space(12.);
                for (label, value) in [("Password", &mut draft.password), ("Repeat password", &mut draft.confirm)] {
                    focus |= ui::field(ui, label, |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut **value)
                                .margin(ui::FIELD_MARGIN)
                                .password(true)
                                .desired_width(f32::INFINITY)
                                .char_limit(127),
                        )
                    })
                    .has_focus();
                }
                let length = draft.password.chars().count();
                if length > 0 && length < 8 {
                    ui.label(
                        RichText::new("A password of 8 or more characters is much harder to guess.")
                            .size(11.5)
                            .color(theme::warning()),
                    );
                }
                if let Some(e) = &draft.error {
                    ui.label(RichText::new(e).size(12.).color(theme::danger()));
                }
                ui.label(
                    RichText::new("Keep the password somewhere safe: without it the copy cannot be opened, by you or anyone. People who know it can print, copy and edit.")
                        .size(11.5)
                        .color(theme::muted()),
                );
                ui.add_space(12.);
                let half = (ui.available_width() - 10.) / 2.;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.;
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        cancel = ui::dialog_button(ui, "Cancel", ui::DialogTone::Secondary).clicked();
                    });
                    ui.allocate_ui(vec2(half, 32.), |ui| {
                        save = ui::dialog_button(ui, "Save a Copy…", ui::DialogTone::Primary).clicked();
                    });
                });
            });
        self.editor.text_editing |= focus;
        if modal.should_close() {
            cancel = true;
        }
        if save {
            if draft.password.is_empty() {
                draft.error = Some("Enter a password".into());
            } else if *draft.password != *draft.confirm {
                draft.error = Some("The passwords are not the same".into());
            } else {
                let password = std::mem::take(&mut draft.password);
                self.protect = None;
                self.save_protected(ctx, Some(password), "protected");
            }
        } else if cancel {
            self.protect = None;
        }
    }

    /// Save a copy with the given password, or with none, and keep that for later copies.
    fn save_protected(
        &mut self,
        ctx: &egui::Context,
        password: Option<Zeroizing<String>>,
        suffix: &str,
    ) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let stem = tab
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        tab.password = password;
        if let Some(s) = &mut tab.session {
            s.mark_unsaved();
        }
        self.save_name = Some(format!("{stem}-{suffix}.pdf"));
        self.save_dialog(ctx);
    }
}

#[cfg(test)]
impl Viewer {
    /// Visual QA: the first tab asks for its password, after a wrong one.
    pub(super) fn preview_password_prompt(&mut self) {
        let source = self.tabs[0].session.as_ref().unwrap().source.clone();
        let mut locked = Locked::new(source);
        locked.error = Some("That password is not right. Try again.".into());
        self.tabs[0].locked = Some(locked);
    }
    pub(super) fn preview_protect(&mut self) {
        self.tabs[0].locked = None;
        self.protect = Some(ProtectDraft {
            password: Zeroizing::new("short".into()),
            confirm: Zeroizing::new("short".into()),
            error: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::App;

    #[test]
    fn a_protected_pdf_opens_with_its_password_and_keeps_it() {
        let temp = tempfile::tempdir().unwrap();
        let font = mypdf_milestone0::document::default_font().unwrap();
        let original = mypdf_milestone0::document::fixture(&font).unwrap();
        let path = temp.path().join("Secret.pdf");
        std::fs::write(&path, security::protect(&original, "doğru-şifre").unwrap()).unwrap();
        let ctx = egui::Context::default();
        let mut app = Viewer::new(
            &eframe::CreationContext::_new_kittest(ctx.clone()),
            vec![path],
        );
        let frame = |app: &mut Viewer| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(pos2(0., 0.), vec2(1100., 800.))),
                    ..Default::default()
                },
                |ctx| app.update(ctx, &mut eframe::Frame::_new_kittest()),
            );
        };
        let until = |app: &mut Viewer, ready: &dyn Fn(&Viewer) -> bool| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            while !ready(app) {
                assert!(std::time::Instant::now() < deadline, "{:?}", app.status);
                frame(app);
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        until(&mut app, &|a| a.tabs[0].locked.is_some());
        assert!(
            app.tabs[0].session.is_none(),
            "nothing is shown before the password"
        );
        // A wrong password is refused and asked again.
        let try_password = |app: &mut Viewer, password: &str| {
            let locked = app.tabs[0].locked.as_mut().unwrap();
            locked.typed = Zeroizing::new(password.into());
            let (tx, rx) = mpsc::channel();
            let encrypted = locked.encrypted.clone();
            let password = password.to_owned();
            std::thread::spawn(move || {
                let _ =
                    tx.send(security::unlock(&encrypted, &password).map_err(|e| format!("{e:#}")));
            });
            locked.rx = Some(rx);
        };
        try_password(&mut app, "yanlış");
        until(&mut app, &|a| {
            a.tabs[0].locked.as_ref().is_some_and(|l| l.error.is_some())
        });
        try_password(&mut app, "doğru-şifre");
        until(&mut app, &|a| a.tabs[0].session.is_some());
        assert!(app.tabs[0].locked.is_none());
        assert_eq!(
            app.tabs[0].password.as_deref().map(String::as_str),
            Some("doğru-şifre")
        );
        assert!(!security::needs_password(
            &app.tabs[0].session.as_ref().unwrap().source
        ));
        // Its content is never written to autosave.
        app.tabs[0].session.as_mut().unwrap().mark_unsaved();
        for _ in 0..3 {
            frame(&mut app);
        }
        assert!(!app.recovery_tracks(app.tabs[0].id));
    }
}
