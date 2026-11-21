use super::{
    process::{self, ProcessPolicy},
    ToolError,
};
use bruhs_models::local_tools::*;
use serde_json::{json, Value};

pub async fn inventory(policy: ProcessPolicy) -> Result<DeviceInventory, ToolError> {
    let android_args = ["devices".into(), "-l".into()];
    let ios_args = [
        "simctl".into(),
        "list".into(),
        "devices".into(),
        "available".into(),
        "--json".into(),
    ];
    let android = process::run("adb", &android_args, policy);
    let ios = process::run("xcrun", &ios_args, policy);
    let (android, ios) = tokio::join!(android, ios);
    let mut inventory = DeviceInventory::default();
    match android.and_then(|bytes| process::text(&bytes)) {
        Ok(text) => inventory.devices.extend(parse_android(&text)),
        Err(error) => inventory.diagnostics.push(format!("Android: {error}. Install Android SDK Platform-Tools and authorize the device's USB debugging request.")),
    }
    match ios.and_then(|bytes| parse_ios(&bytes)) {
        Ok(devices) => inventory.devices.extend(devices),
        Err(error) => inventory.diagnostics.push(format!("iOS: {error}. Install Xcode, select its developer directory and install a simulator runtime.")),
    }
    Ok(inventory)
}

pub(super) fn parse_android(text: &str) -> Vec<Device> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            let state = fields.next()?;
            if id == "List"
                || id.starts_with('*')
                || !matches!(
                    state,
                    "device" | "offline" | "unauthorized" | "recovery" | "bootloader"
                )
            {
                return None;
            }
            let model = fields
                .find_map(|field| field.strip_prefix("model:"))
                .unwrap_or(id);
            Some(Device {
                id: id.into(),
                name: model.replace('_', " "),
                platform: DevicePlatform::Android,
                state: state.into(),
            })
        })
        .collect()
}

pub(super) fn parse_ios(bytes: &[u8]) -> Result<Vec<Device>, ToolError> {
    let value: Value = serde_json::from_slice(bytes)?;
    let runtimes = value["devices"]
        .as_object()
        .ok_or_else(|| ToolError::Invalid("simctl returned no device collection.".into()))?;
    let mut devices = Vec::new();
    for runtime in runtimes.values() {
        for device in runtime.as_array().ok_or_else(|| {
            ToolError::Invalid("simctl returned an invalid runtime device list.".into())
        })? {
            if device["isAvailable"] == false {
                continue;
            }
            let id = device["udid"]
                .as_str()
                .filter(|id| uuid::Uuid::parse_str(id).is_ok())
                .ok_or_else(|| {
                    ToolError::Invalid("simctl returned an invalid simulator identifier.".into())
                })?;
            devices.push(Device {
                id: id.into(),
                name: device["name"].as_str().unwrap_or("iOS simulator").into(),
                platform: DevicePlatform::Ios,
                state: device["state"].as_str().unwrap_or("Unknown").into(),
            });
        }
    }
    Ok(devices)
}

pub async fn execute(
    request: DeviceCommand,
    policy: ProcessPolicy,
) -> Result<ToolReply, ToolError> {
    // The selected id must be a freshly enumerated device, never a free-form CLI option.
    let listed = inventory(policy).await?;
    let device = listed.devices.iter().find(|device| device.id == request.device_id && device.platform == request.platform)
        .ok_or_else(|| ToolError::Invalid("This device is no longer connected or available. Refresh the device list and select it again.".into()))?;
    if request.platform == DevicePlatform::Android && device.state != "device" {
        return Err(ToolError::Unavailable(format!(
            "Android device {} is {}. Authorize or reconnect it before controlling it.",
            device.name, device.state
        )));
    }
    match request.platform {
        DevicePlatform::Android => android(&request.device_id, request.command, policy).await,
        DevicePlatform::Ios => ios(&request.device_id, request.command, policy).await,
    }
}

async fn android(
    id: &str,
    action: DeviceAction,
    policy: ProcessPolicy,
) -> Result<ToolReply, ToolError> {
    let is_screenshot = matches!(action, DeviceAction::Screenshot);
    let args = android_args(id, &action)?;
    let bytes = process::run("adb", &args, policy).await?;
    if is_screenshot {
        super::screenshot(bytes, crate::uploads::write_upload).await
    } else {
        Ok(ToolReply {
            value: json!({"output":process::text(&bytes)?}),
            image: None,
        })
    }
}

pub(super) fn android_args(id: &str, action: &DeviceAction) -> Result<Vec<String>, ToolError> {
    if id.is_empty()
        || id.starts_with('-')
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
    {
        return Err(ToolError::Invalid(
            "Invalid Android device identifier.".into(),
        ));
    }
    let mut args = vec!["-s".into(), id.into()];
    let command = match action {
        DeviceAction::Boot => return Err(ToolError::Unavailable("Start the Android emulator in Android Studio, then refresh Devices. ADB cannot power on a disconnected device.".into())),
        DeviceAction::Shutdown if id.starts_with("emulator-") => vec!["emu".into(), "kill".into()],
        DeviceAction::Shutdown => return Err(ToolError::Invalid("Bruhs only shuts down Android emulators. Use the physical device's power controls.".into())),
        DeviceAction::Screenshot => vec!["exec-out".into(), "screencap".into(), "-p".into()],
        DeviceAction::Tap { x, y } => vec!["shell".into(), "input".into(), "tap".into(), x.to_string(), y.to_string()],
        // adb reconstructs shell arguments on the device. Quoting is necessary even
        // when the host process itself receives an argv vector.
        DeviceAction::Type { text } => vec!["shell".into(), "input".into(), "text".into(), shell_literal(&text.replace(' ', "%s"))],
        DeviceAction::Home => vec!["shell".into(), "input".into(), "keyevent".into(), "3".into()],
        DeviceAction::Back => vec!["shell".into(), "input".into(), "keyevent".into(), "4".into()],
        DeviceAction::OpenUrl { url } => {
            validate_browser_url(url).map_err(|error| ToolError::Invalid(error.into()))?;
            vec!["shell".into(), "am".into(), "start".into(), "-a".into(), "android.intent.action.VIEW".into(), "-d".into(), shell_literal(url)]
        }
    };
    args.extend(command);
    Ok(args)
}

fn shell_literal(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

async fn ios(
    id: &str,
    action: DeviceAction,
    policy: ProcessPolicy,
) -> Result<ToolReply, ToolError> {
    uuid::Uuid::parse_str(id)
        .map_err(|_| ToolError::Invalid("Invalid iOS simulator identifier.".into()))?;
    if matches!(action, DeviceAction::Screenshot) {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("screenshot.png");
        process::run(
            "xcrun",
            &[
                "simctl".into(),
                "io".into(),
                id.into(),
                "screenshot".into(),
                path.to_string_lossy().into_owned(),
            ],
            policy,
        )
        .await?;
        let metadata = tokio::fs::metadata(&path).await?;
        if metadata.len() > u64::try_from(policy.max_output_bytes).unwrap_or(u64::MAX) {
            return Err(ToolError::Unavailable(
                "The simulator screenshot exceeds the capture budget.".into(),
            ));
        }
        return super::screenshot(tokio::fs::read(&path).await?, crate::uploads::write_upload).await;
    }
    let operation = match action {
        DeviceAction::Boot => vec!["boot".into(), id.into()],
        DeviceAction::Shutdown => vec!["shutdown".into(), id.into()],
        DeviceAction::OpenUrl { url } => {
            validate_browser_url(&url).map_err(|error| ToolError::Invalid(error.into()))?;
            vec!["openurl".into(), id.into(), url]
        }
        DeviceAction::Tap { x, y } => {
            return idb(id, vec!["tap".into(), x.to_string(), y.to_string()], policy).await;
        }
        DeviceAction::Type { text } => return idb(id, vec!["text".into(), text], policy).await,
        DeviceAction::Home => return idb(id, vec!["button".into(), "HOME".into()], policy).await,
        DeviceAction::Back => {
            return Err(ToolError::Invalid(
                "iOS has no system Back button. Inspect the screen and tap the app's back control."
                    .into(),
            ));
        }
        DeviceAction::Screenshot => unreachable!("screenshots return above"),
    };
    let mut args = vec!["simctl".into()];
    args.extend(operation);
    let bytes = process::run("xcrun", &args, policy).await?;
    Ok(ToolReply {
        value: json!({"output":process::text(&bytes)?}),
        image: None,
    })
}

async fn idb(
    id: &str,
    operation: Vec<String>,
    policy: ProcessPolicy,
) -> Result<ToolReply, ToolError> {
    // idb's UI command grammar and --udid placement are documented at
    // https://fbidb.io/idb/guided-tour/ and /docs/idb/ui/ (checked 2026-11-13).
    let mut args = vec!["ui".into()];
    args.extend(operation);
    args.extend(["--udid".into(), id.into()]);
    let bytes = process::run("idb", &args, policy).await.map_err(|error| {
        ToolError::Unavailable(format!(
            "iOS touch or typing requires the installed idb CLI and companion: {error}"
        ))
    })?;
    Ok(ToolReply {
        value: json!({"output":process::text(&bytes)?}),
        image: None,
    })
}
