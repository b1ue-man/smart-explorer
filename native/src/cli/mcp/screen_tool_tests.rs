//! Acceptance of the MCP tool `screen` (SCR-M4): its schema as the catalog
//! publishes it, the image answer of a still picture, and the refusal of
//! calls that name no device or an action the tool does not have.
use super::*;
use crate::screen::core::pixel_convert::BgraImage;
use base64::Engine as _;
use serde_json::json;

fn call(value: Value) -> Result<CallToolResult, String> {
    let map = value.as_object().unwrap().clone();
    screen(&Args::new(Some(&map)))
}

#[test]
fn screen_task_mcp_screen_tool_schema() {
    let properties = properties();
    let mut names: Vec<&str> = properties
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "action",
            "device",
            "max_bytes",
            "max_edge",
            "probe",
            "source",
            "wait"
        ]
    );
    assert_eq!(
        properties["action"]["enum"],
        json!(["devices", "probe", "windows", "snapshot"])
    );
    for integer in ["max_edge", "max_bytes", "wait"] {
        assert_eq!(properties[integer]["type"], "integer", "{integer}");
    }
    assert_eq!(properties["probe"]["type"], "boolean");
    assert!(DESCRIPTION.len() <= 2_048);
    for fact in ["approval_pending", "1568", "no live video", "`wait`"] {
        assert!(DESCRIPTION.contains(fact), "{fact}");
    }
    // The tool as the catalog publishes it: closed object, `action`
    // required, read-only (nothing on any device changes).
    let tool = super::super::catalog::tools()
        .into_iter()
        .find(|tool| tool.name == "screen")
        .expect("the catalog lists the tool `screen`");
    assert_eq!(tool.input_schema["type"], "object");
    assert_eq!(tool.input_schema["additionalProperties"], false);
    assert_eq!(tool.input_schema["required"], json!(["action"]));
    assert_eq!(tool.input_schema["properties"], properties);
    assert_eq!(tool.description.as_deref(), Some(DESCRIPTION));
    let annotations = tool.annotations.as_ref().expect("annotations");
    assert_eq!(annotations.read_only_hint, Some(true));
}

#[test]
fn screen_task_mcp_snapshot_image_content() {
    let size = FrameSize::new(320, 200);
    let mut picture = BgraImage::new(size).unwrap();
    for y in 0..size.height {
        for (x, pixel) in picture.row_mut(y).chunks_exact_mut(4).enumerate() {
            pixel.copy_from_slice(&[x as u8, y as u8, 0x40, 255]);
        }
    }
    let fitted = fit(&picture, Some(160), None).unwrap();
    let result = image_result(&fitted, "monitor:DISPLAY1", "Laptop");
    assert_eq!(result.is_error, Some(false));
    let images: Vec<_> = result
        .content
        .iter()
        .filter_map(|block| block.as_image())
        .collect();
    assert_eq!(images.len(), 1, "exactly one image block");
    assert_eq!(images[0].mime_type, "image/png");
    let png = base64::engine::general_purpose::STANDARD
        .decode(&images[0].data)
        .unwrap();
    assert_eq!(&png[..4], &[0x89, 0x50, 0x4E, 0x47]);
    assert_eq!(png, fitted.png);
    // The text block and the structured content carry the facts, not the
    // picture.
    let meta = result.structured_content.clone().unwrap();
    assert_eq!(
        meta,
        json!({
            "width": 160,
            "height": 100,
            "bytes": png.len(),
            "source": "monitor:DISPLAY1",
            "device": "Laptop"
        })
    );
    let texts: Vec<_> = result
        .content
        .iter()
        .filter_map(|block| block.as_text())
        .collect();
    assert_eq!(texts.len(), 1);
    assert_eq!(serde_json::from_str::<Value>(&texts[0].text).unwrap(), meta);
}

#[test]
fn mcp_task_screen_refuses_unnamed_devices_and_unknown_actions() {
    for action in ["probe", "windows", "snapshot"] {
        let error = call(json!({"action": action})).unwrap_err();
        assert!(error.contains("`device`"), "{action}: {error}");
    }
    let error = call(json!({"action": "stream"})).unwrap_err();
    assert!(error.contains("unknown action `stream`"), "{error}");
    assert!(call(json!({})).unwrap_err().contains("`action`"));
    let error = call(json!({"action": "snapshot", "device": "x", "source": "screen"}));
    assert!(error.is_err());
}
