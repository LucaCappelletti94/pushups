//! Extraction of the Android clients from a parsed `google-services.json`.

use std::borrow::Cow;
use std::path::Path;

use serde_json::Value;

/// One Android client of a `google-services.json`.
///
/// The project-level fields borrow from the parsed value.
#[derive(Debug)]
pub(crate) struct Client<'value> {
    /// `client_info.android_client_info.package_name`
    pub(crate) package_name: String,
    /// `client_info.mobilesdk_app_id`
    pub(crate) application_id: String,
    /// `api_key[0].current_key`
    pub(crate) api_key: String,
    /// `project_info.project_number`, the GCM sender id
    pub(crate) gcm_sender_id: Cow<'value, str>,
    /// `project_info.project_id`
    pub(crate) project_id: Cow<'value, str>,
}

/// Reads `path` and parses it as a `google-services.json`.
pub(crate) fn read_value(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("cannot read {}: {}", path.display(), error))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not valid JSON: {}", path.display(), error))
}

/// Extracts the Android clients from a parsed `google-services.json`.
pub(crate) fn clients_from_json(value: &Value) -> Result<Vec<Client<'_>>, String> {
    let clients = value
        .get("client")
        .and_then(Value::as_array)
        .ok_or_else(|| "the file has no \"client\" array".to_owned())?;
    if clients.is_empty() {
        return Err("the file has no clients".to_owned());
    }
    let project = value
        .get("project_info")
        .ok_or_else(|| "the file has no \"project_info\" object".to_owned())?;
    let gcm_sender_id = text(
        "the file",
        project,
        "project_number",
        "project_info.project_number",
    )?;
    let project_id = text("the file", project, "project_id", "project_info.project_id")?;

    clients
        .iter()
        .enumerate()
        .map(|(index, client)| {
            let subject = format!("client {index}");
            let info = client
                .get("client_info")
                .ok_or_else(|| format!("{subject} has no \"client_info\" object"))?;
            let android = info
                .get("android_client_info")
                .ok_or_else(|| format!("{subject} has no \"android_client_info\" object"))?;
            let package_name = text(
                &subject,
                android,
                "package_name",
                "client_info.android_client_info.package_name",
            )?
            .into_owned();
            let application_id = text(
                &subject,
                info,
                "mobilesdk_app_id",
                "client_info.mobilesdk_app_id",
            )?
            .into_owned();
            let keys = client
                .get("api_key")
                .and_then(Value::as_array)
                .ok_or_else(|| format!("{subject} has no \"api_key\" array"))?;
            let first = keys
                .first()
                .ok_or_else(|| format!("{subject} has an empty \"api_key\" array"))?;
            let api_key =
                text(&subject, first, "current_key", "api_key[0].current_key")?.into_owned();
            Ok(Client {
                package_name,
                application_id,
                api_key,
                gcm_sender_id: gcm_sender_id.clone(),
                project_id: project_id.clone(),
            })
        })
        .collect()
}

/// Reads `key` out of `object` as text, or reports it missing under `path`.
fn text<'value>(
    subject: &str,
    object: &'value Value,
    key: &str,
    path: &str,
) -> Result<Cow<'value, str>, String> {
    match object.get(key) {
        Some(Value::String(value)) => Ok(value.as_str().into()),
        Some(Value::Number(value)) => Ok(value.to_string().into()),
        Some(_) => Err(format!(
            "{subject} has {path} but it is neither a string nor a number"
        )),
        None => Err(format!("{subject} has no {path}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const ONE_CLIENT: &str = r#"{
        "project_info": {
            "project_number": "1",
            "project_id": "demo"
        },
        "client": [
            {
                "client_info": {
                    "mobilesdk_app_id": "1:1:android:1",
                    "android_client_info": { "package_name": "com.example.demo" }
                },
                "api_key": [ { "current_key": "key-one" } ]
            }
        ]
    }"#;

    const TWO_CLIENTS: &str = r#"{
        "project_info": {
            "project_number": 42,
            "project_id": "demo"
        },
        "client": [
            {
                "client_info": {
                    "mobilesdk_app_id": "1:42:android:a",
                    "android_client_info": { "package_name": "com.example.first" }
                },
                "api_key": [ { "current_key": "key-a" } ]
            },
            {
                "client_info": {
                    "mobilesdk_app_id": "1:42:android:b",
                    "android_client_info": { "package_name": "com.example.second" }
                },
                "api_key": [ { "current_key": "key-b" } ]
            }
        ]
    }"#;

    #[test]
    fn one_client_extracts_the_fields() {
        let value: Value = serde_json::from_str(ONE_CLIENT).expect("fixture is valid JSON");
        let clients = clients_from_json(&value).expect("fixture parses");
        assert_eq!(clients.len(), 1);
        assert_eq!(clients[0].package_name, "com.example.demo");
        assert_eq!(clients[0].application_id, "1:1:android:1");
        assert_eq!(clients[0].api_key, "key-one");
        assert_eq!(clients[0].gcm_sender_id, "1");
        assert_eq!(clients[0].project_id, "demo");
    }

    #[test]
    fn two_clients_extract_the_fields() {
        let value: Value = serde_json::from_str(TWO_CLIENTS).expect("fixture is valid JSON");
        let clients = clients_from_json(&value).expect("fixture parses");
        assert_eq!(clients.len(), 2);
        assert_eq!(clients[0].package_name, "com.example.first");
        assert_eq!(clients[1].package_name, "com.example.second");
        assert_eq!(clients[0].api_key, "key-a");
        assert_eq!(clients[1].api_key, "key-b");
        // a numeric project_number is stringified for the GCM sender id
        assert_eq!(clients[0].gcm_sender_id, "42");
        assert_eq!(clients[1].gcm_sender_id, "42");
        assert_eq!(clients[1].project_id, "demo");
    }

    #[test]
    fn missing_api_key_names_the_client() {
        let value: Value = serde_json::from_str(
            r#"{
            "project_info": { "project_number": "1", "project_id": "demo" },
            "client": [
                {
                    "client_info": {
                        "mobilesdk_app_id": "1:1:android:1",
                        "android_client_info": { "package_name": "com.example.demo" }
                    }
                }
            ]
        }"#,
        )
        .expect("fixture is valid JSON");
        let error = clients_from_json(&value).expect_err("the client has no api_key");
        assert!(error.contains("client 0"));
        assert!(error.contains("api_key"));
    }

    #[test]
    fn no_clients_is_an_error() {
        let value: Value = serde_json::from_str(
            r#"{ "project_info": { "project_number": "1", "project_id": "demo" }, "client": [] }"#,
        )
        .expect("fixture is valid JSON");
        let error = clients_from_json(&value).expect_err("there are no clients");
        assert!(error.contains("no clients"));
    }

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pushups-macros-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir is creatable");
        dir
    }

    #[test]
    fn missing_file_names_the_path() {
        let path = scratch_dir().join("missing.json");
        let error = read_value(&path).expect_err("the file does not exist");
        assert!(error.contains(path.to_str().expect("path is UTF-8")));
    }

    #[test]
    fn malformed_json_names_the_file() {
        let path = scratch_dir().join("broken.json");
        std::fs::write(&path, "{ not json").expect("the file is written");
        let error = read_value(&path).expect_err("the file is not valid JSON");
        assert!(error.contains("not valid JSON"));
        assert!(error.contains(path.to_str().expect("path is UTF-8")));
    }
}
