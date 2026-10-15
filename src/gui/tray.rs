//! The tray icon and its menu (Kill Minecraft, Toggle Console, Open Launcher Folder, Quit), like the Java edition.
//! Linux uses the StatusNotifier protocol (no GTK needed); Windows and macOS use the system tray.

use super::{Msg, Notifier};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCmd {
    Toggle,
    KillMinecraft,
    ToggleConsole,
    OpenFolder,
    Quit,
}

const ITEMS: [(&str, TrayCmd); 4] = [
    ("Kill Minecraft", TrayCmd::KillMinecraft),
    ("Toggle Console", TrayCmd::ToggleConsole),
    ("Open Launcher Folder", TrayCmd::OpenFolder),
    ("Quit", TrayCmd::Quit),
];

/// The launcher logo scaled to a tray-sized square, as RGBA.
fn icon_rgba(size: u32) -> Option<(Vec<u8>, u32, u32)> {
    let img = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?;
    let img = img.resize(size, size, image::imageops::FilterType::Lanczos3).to_rgba8();
    let (w, h) = (img.width(), img.height());
    Some((img.into_raw(), w, h))
}

#[cfg(target_os = "linux")]
pub struct Tray {
    handle: Option<ksni::Handle<LinuxTray>>,
    rt: tokio::runtime::Handle,
}

#[cfg(target_os = "linux")]
pub struct LinuxTray {
    n: Notifier,
}

#[cfg(target_os = "linux")]
impl ksni::Tray for LinuxTray {
    fn id(&self) -> String {
        "blockforge".into()
    }

    fn title(&self) -> String {
        crate::NAME.into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        match icon_rgba(64) {
            Some((mut data, w, h)) => {
                for px in data.chunks_exact_mut(4) {
                    px.rotate_right(1); // RGBA to ARGB
                }
                vec![ksni::Icon { width: w as i32, height: h as i32, data }]
            }
            None => Vec::new(),
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.n.send(Msg::Tray(TrayCmd::Toggle));
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        ITEMS
            .iter()
            .map(|&(label, cmd)| {
                StandardItem {
                    label: label.into(),
                    activate: Box::new(move |t: &mut LinuxTray| t.n.send(Msg::Tray(cmd))),
                    ..Default::default()
                }
                .into()
            })
            .collect()
    }
}

#[cfg(target_os = "linux")]
impl Tray {
    /// Start the tray. Returns a handle that removes the icon again when dropped; quietly does nothing useful on
    /// desktops that have no tray (the answer arrives later through `Msg::Toast`).
    pub fn start(rt: &tokio::runtime::Runtime, n: Notifier) -> Tray {
        use ksni::TrayMethods;
        let (tx, rx) = std::sync::mpsc::channel();
        let tray = LinuxTray { n: n.clone() };
        rt.spawn(async move {
            match tray.spawn().await {
                Ok(h) => {
                    let _ = tx.send(h);
                }
                Err(e) => n.send(Msg::Toast(format!("No tray available on this desktop ({e})"), false)),
            }
        });
        // the handle arrives a moment later; `stop` waits briefly for it if it has to
        let handle = rx.recv_timeout(std::time::Duration::from_millis(300)).ok();
        Tray { handle, rt: rt.handle().clone() }
    }

    pub fn stop(mut self) {
        if let Some(h) = self.handle.take() {
            self.rt.spawn(async move {
                h.shutdown().await;
            });
        }
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Tray {
    pub fn start(_rt: &tokio::runtime::Runtime, n: Notifier) -> Tray {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
        let menu = Menu::new();
        let mut ids = Vec::new();
        for (label, cmd) in ITEMS {
            let item = MenuItem::new(label, true, None);
            ids.push((item.id().clone(), cmd));
            let _ = menu.append(&item);
        }
        let n1 = n.clone();
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some((_, cmd)) = ids.iter().find(|(id, _)| *id == e.id) {
                n1.send(Msg::Tray(*cmd));
            }
        }));
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                n.send(Msg::Tray(TrayCmd::Toggle));
            }
        }));
        let mut builder = TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip(crate::NAME);
        if let Some((rgba, w, h)) = icon_rgba(64) {
            if let Ok(icon) = Icon::from_rgba(rgba, w, h) {
                builder = builder.with_icon(icon);
            }
        }
        Tray { _icon: builder.build().expect("tray icon") }
    }

    pub fn stop(self) {}
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub struct Tray;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
impl Tray {
    pub fn start(_rt: &tokio::runtime::Runtime, _n: Notifier) -> Tray {
        Tray
    }

    pub fn stop(self) {}
}
