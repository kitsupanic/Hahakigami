//! Disposable native panel exercise. Never launches or reads the user's app or its data.
#[path = "../src/quick_look.rs"]
mod quick_look;
#[cfg(target_os = "macos")]
mod smoke {
    use super::quick_look::{self, Registry};
    use objc2::{msg_send, rc::Retained, runtime::ProtocolObject, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSWindow};
    use objc2_foundation::NSURL;
    use objc2_quick_look_ui::{QLPreviewItem, QLPreviewPanel};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
    thread_local! { static RETAINED_ITEM: std::cell::RefCell<Option<Retained<ProtocolObject<dyn QLPreviewItem>>>> = const { std::cell::RefCell::new(None) }; }
    async fn main_call<T: Send + 'static>(
        app: &AppHandle,
        task: impl FnOnce() -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        app.run_on_main_thread(move || {
            let _ = tx.send(task());
        })
        .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || rx.recv().map_err(|e| e.to_string())?)
            .await
            .map_err(|e| e.to_string())?
    }
    async fn pause() {
        tauri::async_runtime::spawn_blocking(|| {
            std::thread::sleep(std::time::Duration::from_millis(150))
        })
        .await
        .unwrap();
    }
    async fn native_next(app: &AppHandle, window: &WebviewWindow) -> Result<usize, String> {
        let window = window.clone();
        main_call(app, move || unsafe {
            let raw = window.ns_window().map_err(|e| e.to_string())?;
            let window = &*(raw as *const NSWindow);
            Ok(window
                .firstResponder()
                .and_then(|anchor| anchor.nextResponder())
                .map_or(0, |r| &*r as *const _ as usize))
        })
        .await
    }
    async fn verify_visible(app: &AppHandle) -> Result<(), String> {
        main_call(app, || unsafe {
            let panel = QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap())
                .ok_or("no panel")?;
            if !panel.isVisible()
                || panel.currentController().is_none()
                || panel.dataSource().is_none()
            {
                return Err("panel ownership/data source missing".into());
            }
            let item = panel.currentPreviewItem().ok_or("no preview item")?;
            let url: Option<objc2::rc::Retained<NSURL>> = msg_send![&*item, previewItemURL];
            RETAINED_ITEM.with(|slot| *slot.borrow_mut() = Some(item));
            if url.is_none() {
                return Err("item URL missing".into());
            }
            Ok(())
        })
        .await
    }
    fn pdf() -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>",
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R >>",
            "<< /Length 0 >>\nstream\n\nendstream",
        ];
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0];
        for (i, obj) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).bytes());
        }
        let xref = bytes.len();
        bytes.extend(b"xref\n0 5\n0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            bytes.extend(format!("{offset:010} 00000 n \n").bytes());
        }
        bytes.extend(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").bytes(),
        );
        bytes
    }
    async fn exercise(
        app: AppHandle,
        window: WebviewWindow,
        fixture: tempfile::TempDir,
    ) -> Result<(), String> {
        let registry = Arc::new(Registry::default());
        let checks = Arc::new(AtomicUsize::new(0));

        main_call(&app, || {
            #[allow(deprecated)]
            NSApplication::sharedApplication(MainThreadMarker::new().unwrap())
                .activateIgnoringOtherApps(true);
            Ok(())
        })
        .await?;
        window.set_focus().map_err(|e| e.to_string())?;
        pause().await;
        let original = native_next(&app, &window).await?;
        for (id, name) in [(1, "fixture.txt"), (2, "fixture.png"), (3, "fixture.pdf")] {
            let path = fixture.path().join(name);
            let counter = checks.clone();
            let lifetime = Arc::new(());
            let weak = Arc::downgrade(&lifetime);
            let check = Arc::new(move || {
                let _keep_alive = &lifetime;
                counter.fetch_add(1, Ordering::Relaxed);
                Ok(())
            });
            let rev = registry.begin()?;
            let status = quick_look::present(
                app.clone(),
                window.clone(),
                registry.clone(),
                rev,
                "smoke".into(),
                id,
                path.clone(),
                check.clone(),
                Arc::new(|| Ok(())),
            )
            .await?;
            if !status.open {
                return Err("preview failed to open".into());
            }
            pause().await;
            verify_visible(&app).await?;
            if checks.load(Ordering::Relaxed) < 2 {
                return Err("URL handoff did not revalidate".into());
            }
            // Same current item toggles the panel closed.
            let rev = registry.begin()?;
            if quick_look::present(
                app.clone(),
                window.clone(),
                registry.clone(),
                rev,
                "smoke".into(),
                id,
                path,
                check,
                Arc::new(|| Ok(())),
            )
            .await?
            .open
            {
                return Err("repeat preview failed to toggle".into());
            }
            if registry.status().revision <= rev {
                return Err("toggle close status failed to advance revision".into());
            }
            if native_next(&app, &window).await? != original {
                return Err("responder chain not restored after toggle".into());
            }
            if weak.upgrade().is_some() {
                return Err("closed preview retained its Check captures".into());
            }
            main_call(&app, || {
                RETAINED_ITEM.with(|slot| {
                    if let Some(item) = slot.borrow_mut().take() {
                        let url: Option<Retained<NSURL>> =
                            unsafe { msg_send![&*item, previewItemURL] };
                        if url.is_some() {
                            return Err("closed retained item still exposed a URL".into());
                        }
                    }
                    Ok(())
                })
            })
            .await?;
        }
        // R4: mutate a disposable target between the initial validation and an
        // actual wait for the previous panel's fade. Every retry must refresh.
        let old_rev = registry.begin()?;
        quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            old_rev,
            "smoke".into(),
            1,
            fixture.path().join("fixture.txt"),
            Arc::new(|| Ok(())),
            Arc::new(|| Ok(())),
        )
        .await?;
        let retry_path = fixture.path().join("retry.txt");
        std::fs::write(&retry_path, b"expected").map_err(|e| e.to_string())?;
        let refresh_calls = Arc::new(AtomicUsize::new(0));
        let observed = refresh_calls.clone();
        let refreshed_path = retry_path.clone();
        let refresh = Arc::new(move || {
            if MainThreadMarker::new().is_some() {
                return Err("filesystem refresh ran on UI thread".into());
            }
            let call = observed.fetch_add(1, Ordering::SeqCst);
            if std::fs::read(&refreshed_path).map_err(|e| e.to_string())? != b"expected" {
                return Err("retry replacement detected".into());
            }
            if call == 0 {
                std::fs::write(&refreshed_path, b"replacement").map_err(|e| e.to_string())?;
            }
            Ok(())
        });
        let retry_rev = registry.begin()?;
        let error = quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            retry_rev,
            "smoke".into(),
            4,
            retry_path,
            Arc::new(|| Ok(())),
            refresh,
        )
        .await
        .err()
        .ok_or("changed target survived retry refresh")?;
        if !error.contains("retry replacement detected") || refresh_calls.load(Ordering::SeqCst) < 2
        {
            return Err(format!(
                "retry refresh regression did not exercise delayed validation: {error}"
            ));
        }
        main_call(&app, || unsafe {
            if QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap())
                .unwrap()
                .isVisible()
            {
                return Err("failed retry left old preview visible".into());
            }
            Ok(())
        })
        .await?;
        eprintln!("quick_look_smoke: R4 delayed refresh rejection PASS");
        let path = fixture.path().join("fixture.txt");
        let rev = registry.begin()?;
        quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            rev,
            "smoke".into(),
            1,
            path.clone(),
            Arc::new(|| Ok(())),
            Arc::new(|| Ok(())),
        )
        .await?;
        if std::env::var_os("HAH_QUICK_LOOK_SMOKE_EXTERNAL_KEYS").is_some() {
            eprintln!("quick_look_smoke: READY physical Escape/fullscreen fixture");
            for _ in 0..200 {
                if !registry.status().open {
                    break;
                }
                pause().await;
            }
        } else {
            main_call(&app, || unsafe {
                QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap())
                    .unwrap()
                    .performClose(None);
                Ok(())
            })
            .await?;
        }
        for _ in 0..40 {
            if !registry.status().open {
                break;
            }
            pause().await;
        }
        if registry.status().open {
            return Err("native close/Escape did not close registry".into());
        }
        if registry.status().revision <= rev {
            return Err("native close event failed to advance revision".into());
        }
        if native_next(&app, &window).await? != original {
            return Err("responder chain not restored after Escape".into());
        }
        let rev = registry.begin()?;
        quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            rev,
            "smoke".into(),
            1,
            path.clone(),
            Arc::new(|| Ok(())),
            Arc::new(|| Ok(())),
        )
        .await?;
        let close_a = tauri::async_runtime::spawn(quick_look::close(app.clone(), registry.clone()));
        let close_b = tauri::async_runtime::spawn(quick_look::close(app.clone(), registry.clone()));
        let _older = close_a.await.map_err(|e| e.to_string())??;
        let status = close_b.await.map_err(|e| e.to_string())??;
        main_call(&app, || unsafe {
            let panel =
                QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap()).unwrap();
            if panel.isVisible() || panel.isInFullScreenMode() {
                return Err("concurrent close returned before dismissal".into());
            }
            Ok(())
        })
        .await?;
        if status.open || native_next(&app, &window).await? != original {
            return Err("awaited close failed".into());
        }
        let stale = registry.begin()?;
        registry.invalidate();
        if quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            stale,
            "smoke".into(),
            1,
            path.clone(),
            Arc::new(|| Ok(())),
            Arc::new(|| Ok(())),
        )
        .await
        .is_ok()
        {
            return Err("stale preview accepted".into());
        }
        let rev = registry.begin()?;
        quick_look::present(
            app.clone(),
            window.clone(),
            registry.clone(),
            rev,
            "smoke".into(),
            1,
            path,
            Arc::new(|| Ok(())),
            Arc::new(|| Ok(())),
        )
        .await?;
        // R3: close the host while the preview is fullscreen, then request close
        // after Tauri has removed the host handle. Success gates cleanup on hiding.
        main_call(&app, || unsafe {
            let panel =
                QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap()).unwrap();
            if !panel.enterFullScreenMode_withOptions(None, None) {
                return Err("fullscreen entry failed".into());
            }
            Ok(())
        })
        .await?;
        pause().await;
        if std::env::var_os("HAH_QUICK_LOOK_SMOKE_ENTERING_FULLSCREEN").is_none() {
            tauri::async_runtime::spawn_blocking(|| {
                std::thread::sleep(std::time::Duration::from_secs(2))
            })
            .await
            .unwrap();
        }
        eprintln!("quick_look_smoke: R3 fullscreen host teardown starting");
        window.close().map_err(|e| e.to_string())?;
        for _ in 0..40 {
            if app.get_webview_window("main").is_none() {
                break;
            }
            pause().await;
        }
        if app.get_webview_window("main").is_some() {
            return Err("host was not destroyed".into());
        }
        quick_look::close(app.clone(), registry.clone()).await?;
        main_call(&app, || unsafe {
            let panel =
                QLPreviewPanel::sharedPreviewPanel(MainThreadMarker::new().unwrap()).unwrap();
            if panel.isVisible() || panel.isInFullScreenMode() {
                return Err("hostless close allowed cleanup before actual dismissal".into());
            }
            Ok(())
        })
        .await?;
        if registry.status().open {
            return Err("host-window close failed to invalidate".into());
        }
        eprintln!("quick_look_smoke: PASS text/image/PDF, repeated toggle, URL revalidation, native close (physical Escape in external-key mode), capture release, concurrent awaited close, stale request, fullscreen hostless close gating, retry FS refresh, monotonic close status, responder restoration");
        Ok(())
    }
    pub fn run() {
        let fixture = tempfile::tempdir().unwrap();
        std::fs::write(
            fixture.path().join("fixture.txt"),
            b"Quick Look disposable smoke fixture",
        )
        .unwrap();
        std::fs::write(fixture.path().join("fixture.pdf"), pdf()).unwrap();
        // Valid one-pixel PNG, exclusively in the disposable fixture.
        std::fs::write(
            fixture.path().join("fixture.png"),
            [
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0,
                1, 8, 6, 0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248,
                207, 192, 240, 31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68,
                174, 66, 96, 130,
            ],
        )
        .unwrap();
        let mut context = tauri::generate_context!();
        context.config_mut().identifier = "app.hahakigami.quicklook-smoke".into();
        context.config_mut().app.windows.clear();
        let app = tauri::Builder::default()
            .setup(move |app| {
                let window = WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External("about:blank".parse().unwrap()),
                )
                .title("Quick Look smoke fixture")
                .build()?;
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let result = exercise(handle.clone(), window, fixture).await;
                    if let Err(error) = &result {
                        eprintln!("quick_look_smoke: FAIL {error}");
                    }
                    handle.exit(if result.is_ok() { 0 } else { 1 });
                });
                Ok(())
            })
            .build(context)
            .unwrap();
        let code = app.run_return(|_app, event| {
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
            }
        });
        std::process::exit(code);
    }
}
fn main() {
    #[cfg(target_os = "macos")]
    smoke::run();
    #[cfg(not(target_os = "macos"))]
    eprintln!("Native smoke requires macOS");
}
