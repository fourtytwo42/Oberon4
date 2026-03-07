use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    OpenConsole,
    RestartApp,
    Exit,
}

pub trait TrayCommandHandler: Send + Sync {
    fn handle(&self, action: TrayAction);
}

#[derive(Clone)]
pub struct TrayRuntime {
    console_url: String,
    handler: Arc<dyn TrayCommandHandler>,
}

impl std::fmt::Debug for TrayRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayRuntime")
            .field("console_url", &self.console_url)
            .finish()
    }
}

impl TrayRuntime {
    pub fn new(console_url: impl Into<String>, handler: Arc<dyn TrayCommandHandler>) -> Self {
        Self {
            console_url: console_url.into(),
            handler,
        }
    }

    pub fn native_supported() -> bool {
        cfg!(all(
            feature = "native-tray",
            any(target_os = "windows", target_os = "macos")
        ))
    }

    pub fn left_click(&self) {
        let _ = webbrowser::open(&self.console_url);
        self.handler.handle(TrayAction::OpenConsole);
    }

    pub fn menu_open_console(&self) {
        let _ = webbrowser::open(&self.console_url);
        self.handler.handle(TrayAction::OpenConsole);
    }

    pub fn menu_restart(&self) {
        self.handler.handle(TrayAction::RestartApp);
    }

    pub fn menu_exit(&self) {
        self.handler.handle(TrayAction::Exit);
    }

    #[cfg(all(
        feature = "native-tray",
        any(target_os = "windows", target_os = "macos")
    ))]
    pub fn run_native(self) -> Result<(), String> {
        use image::imageops::FilterType;
        use tray_icon::{
            menu::{Menu, MenuEvent, MenuItem},
            Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent,
        };
        #[cfg(target_os = "windows")]
        Self::enable_windows_auto_dark_menus();

        #[cfg(target_os = "windows")]
        let icon_bytes: &[u8] = if Self::windows_prefers_dark_mode() {
            include_bytes!("../../../light-logo.png").as_slice()
        } else {
            include_bytes!("../../../dark-logo.png").as_slice()
        };
        #[cfg(not(target_os = "windows"))]
        let icon_bytes: &[u8] = include_bytes!("../../../dark-logo.png").as_slice();
        let icon_img = image::load_from_memory(icon_bytes)
            .map_err(|e| format!("failed to decode tray logo.png: {e}"))?
            .to_rgba8();
        let icon_resized = image::imageops::resize(&icon_img, 32, 32, FilterType::Lanczos3);
        let icon = Icon::from_rgba(icon_resized.into_raw(), 32, 32).map_err(|e| e.to_string())?;
        let menu = Menu::new();
        let open_item = MenuItem::new("Open Console", true, None);
        let restart_item = MenuItem::new("Restart App", true, None);
        let exit_item = MenuItem::new("Exit", true, None);
        menu.append(&open_item).map_err(|e| e.to_string())?;
        menu.append(&restart_item).map_err(|e| e.to_string())?;
        menu.append(&exit_item).map_err(|e| e.to_string())?;

        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_icon(icon)
            .with_tooltip(concat!(
                "LLM Hotspot v",
                env!("CARGO_PKG_VERSION"),
                " (",
                env!("GIT_HASH"),
                ")"
            ))
            .with_menu_on_left_click(false)
            .build()
            .map_err(|e| e.to_string())?;

        loop {
            if let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = event.id;
                if id == open_item.id() {
                    self.menu_open_console();
                } else if id == restart_item.id() {
                    self.menu_restart();
                } else if id == exit_item.id() {
                    self.menu_exit();
                    break;
                }
            }
            if let Ok(TrayIconEvent::Click {
                button,
                button_state,
                ..
            }) = TrayIconEvent::receiver().try_recv()
            {
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    self.left_click();
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }

        Ok(())
    }

    #[cfg(all(feature = "native-tray", target_os = "windows"))]
    fn enable_windows_auto_dark_menus() {
        // Ask Windows to apply native menu dark/light appearance based on system settings.
        unsafe {
            use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};
            let module = LoadLibraryA(c"uxtheme.dll".as_ptr() as _);
            if module.is_null() {
                return;
            }
            let set_preferred = GetProcAddress(module, c"SetPreferredAppMode".as_ptr() as _);
            if let Some(set_preferred) = set_preferred {
                let set_preferred: extern "system" fn(i32) -> i32 =
                    std::mem::transmute(set_preferred);
                const ALLOW_DARK: i32 = 1;
                let _ = set_preferred(ALLOW_DARK);
            }
            let flush_menu_themes = GetProcAddress(module, c"FlushMenuThemes".as_ptr() as _);
            if let Some(flush_menu_themes) = flush_menu_themes {
                let flush_menu_themes: extern "system" fn() =
                    std::mem::transmute(flush_menu_themes);
                flush_menu_themes();
            }
        }
    }

    #[cfg(all(feature = "native-tray", target_os = "windows"))]
    fn windows_prefers_dark_mode() -> bool {
        use windows_sys::Win32::System::Registry::{
            RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD,
        };
        let subkey: Vec<u16> =
            "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0"
                .encode_utf16()
                .collect();
        let value_name: Vec<u16> = "AppsUseLightTheme\0".encode_utf16().collect();
        let mut data: u32 = 1;
        let mut data_len: u32 = std::mem::size_of::<u32>() as u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                value_name.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                &mut data as *mut u32 as *mut _,
                &mut data_len,
            )
        };
        status == 0 && data == 0
    }

    #[cfg(all(
        feature = "native-tray",
        not(any(target_os = "windows", target_os = "macos"))
    ))]
    pub fn run_native(self) -> Result<(), String> {
        let _ = self;
        Err("native tray backend is not enabled for this platform".to_string())
    }
}

#[derive(Debug, Clone, Default)]
pub struct RecordingHandler {
    actions: Arc<Mutex<Vec<TrayAction>>>,
}

impl RecordingHandler {
    pub fn actions(&self) -> Vec<TrayAction> {
        self.actions
            .lock()
            .expect("recording handler mutex poisoned")
            .clone()
    }
}

impl TrayCommandHandler for RecordingHandler {
    fn handle(&self, action: TrayAction) {
        self.actions
            .lock()
            .expect("recording handler mutex poisoned")
            .push(action);
    }
}
