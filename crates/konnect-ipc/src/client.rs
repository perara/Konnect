//! KiCAD 10 IPC API client using NNG + Protocol Buffers.
//!
//! KiCAD 10 exposes an IPC API over NNG (nanomsg-next-gen) using protobuf messages.
//! The transport is NNG req/rep over IPC (Unix sockets / Windows named pipes).
//!
//! Socket path: set by KICAD_API_SOCKET env var when KiCAD launches a plugin,
//! or can be manually specified.
//!
//! Protocol: ApiRequest envelope containing a google.protobuf.Any body → ApiResponse.

use crate::gen::kiapi;
use crate::types::*;
use anyhow::{Context, Result};
// NNG SetOpt trait is brought in scope automatically by the nng crate's prelude
use prost::Message;
use tracing::{debug, warn};

/// Converts KiCAD nanometers to millimeters.
fn nm_to_mm(nm: i64) -> f64 {
    nm as f64 / 1_000_000.0
}

/// Map a BoardLayer enum integer back to a KiCAD layer name string.
fn layer_enum_to_name(layer: i32) -> &'static str {
    match kiapi::board::types::BoardLayer::try_from(layer) {
        Ok(l) => match l {
            kiapi::board::types::BoardLayer::BlFCu => "F.Cu",
            kiapi::board::types::BoardLayer::BlBCu => "B.Cu",
            kiapi::board::types::BoardLayer::BlIn1Cu => "In1.Cu",
            kiapi::board::types::BoardLayer::BlIn2Cu => "In2.Cu",
            kiapi::board::types::BoardLayer::BlFSilkS => "F.SilkS",
            kiapi::board::types::BoardLayer::BlBSilkS => "B.SilkS",
            kiapi::board::types::BoardLayer::BlFMask => "F.Mask",
            kiapi::board::types::BoardLayer::BlBMask => "B.Mask",
            kiapi::board::types::BoardLayer::BlFPaste => "F.Paste",
            kiapi::board::types::BoardLayer::BlBPaste => "B.Paste",
            kiapi::board::types::BoardLayer::BlFCrtYd => "F.CrtYd",
            kiapi::board::types::BoardLayer::BlBCrtYd => "B.CrtYd",
            kiapi::board::types::BoardLayer::BlFFab => "F.Fab",
            kiapi::board::types::BoardLayer::BlBFab => "B.Fab",
            kiapi::board::types::BoardLayer::BlEdgeCuts => "Edge.Cuts",
            _ => "Unknown",
        },
        Err(_) => "Unknown",
    }
}

/// Wrap a protobuf message into a prost_types::Any with the correct type_url.
fn pack_any<M: Message>(msg: &M, type_name: &str) -> prost_types::Any {
    let mut buf = Vec::new();
    msg.encode(&mut buf).expect("protobuf encode failed");
    prost_types::Any {
        type_url: format!("type.googleapis.com/{}", type_name),
        value: buf,
    }
}

/// Decode a prost_types::Any into a specific protobuf message type.
fn unpack_any<M: Message + Default>(any: &prost_types::Any) -> Result<M> {
    M::decode(any.value.as_slice()).context("Failed to decode protobuf Any body")
}

fn translate_vector(vector: &mut Option<kiapi::common::types::Vector2>, dx_nm: i64, dy_nm: i64) {
    if let Some(vector) = vector {
        vector.x_nm = vector.x_nm.saturating_add(dx_nm);
        vector.y_nm = vector.y_nm.saturating_add(dy_nm);
    }
}

fn translate_poly_line(line: &mut kiapi::common::types::PolyLine, dx_nm: i64, dy_nm: i64) {
    use kiapi::common::types::poly_line_node::Geometry;

    for node in &mut line.nodes {
        match node.geometry.as_mut() {
            Some(Geometry::Point(point)) => {
                point.x_nm = point.x_nm.saturating_add(dx_nm);
                point.y_nm = point.y_nm.saturating_add(dy_nm);
            }
            Some(Geometry::Arc(arc)) => {
                translate_vector(&mut arc.start, dx_nm, dy_nm);
                translate_vector(&mut arc.mid, dx_nm, dy_nm);
                translate_vector(&mut arc.end, dx_nm, dy_nm);
            }
            None => {}
        }
    }
}

fn translate_poly_set(set: &mut Option<kiapi::common::types::PolySet>, dx_nm: i64, dy_nm: i64) {
    let Some(set) = set else {
        return;
    };
    translate_poly_set_value(set, dx_nm, dy_nm);
}

fn translate_poly_set_value(set: &mut kiapi::common::types::PolySet, dx_nm: i64, dy_nm: i64) {
    for polygon in &mut set.polygons {
        if let Some(outline) = &mut polygon.outline {
            translate_poly_line(outline, dx_nm, dy_nm);
        }
        for hole in &mut polygon.holes {
            translate_poly_line(hole, dx_nm, dy_nm);
        }
    }
}

fn translate_text(text: &mut kiapi::common::types::Text, dx_nm: i64, dy_nm: i64) {
    translate_vector(&mut text.position, dx_nm, dy_nm);
}

fn translate_field(field: &mut kiapi::board::types::Field, dx_nm: i64, dy_nm: i64) {
    if let Some(text) = field
        .text
        .as_mut()
        .and_then(|board_text| board_text.text.as_mut())
    {
        translate_text(text, dx_nm, dy_nm);
    }
}

fn translate_graphic_shape(shape: &mut kiapi::common::types::GraphicShape, dx_nm: i64, dy_nm: i64) {
    use kiapi::common::types::graphic_shape::Geometry;

    match shape.geometry.as_mut() {
        Some(Geometry::Segment(segment)) => {
            translate_vector(&mut segment.start, dx_nm, dy_nm);
            translate_vector(&mut segment.end, dx_nm, dy_nm);
        }
        Some(Geometry::Rectangle(rectangle)) => {
            translate_vector(&mut rectangle.top_left, dx_nm, dy_nm);
            translate_vector(&mut rectangle.bottom_right, dx_nm, dy_nm);
        }
        Some(Geometry::Arc(arc)) => {
            translate_vector(&mut arc.start, dx_nm, dy_nm);
            translate_vector(&mut arc.mid, dx_nm, dy_nm);
            translate_vector(&mut arc.end, dx_nm, dy_nm);
        }
        Some(Geometry::Circle(circle)) => {
            translate_vector(&mut circle.center, dx_nm, dy_nm);
            translate_vector(&mut circle.radius_point, dx_nm, dy_nm);
        }
        Some(Geometry::Polygon(polygon)) => {
            translate_poly_set_value(polygon, dx_nm, dy_nm);
        }
        Some(Geometry::Bezier(bezier)) => {
            translate_vector(&mut bezier.start, dx_nm, dy_nm);
            translate_vector(&mut bezier.control1, dx_nm, dy_nm);
            translate_vector(&mut bezier.control2, dx_nm, dy_nm);
            translate_vector(&mut bezier.end, dx_nm, dy_nm);
        }
        None => {}
    }
}

fn translate_dimension(dimension: &mut kiapi::board::types::Dimension, dx_nm: i64, dy_nm: i64) {
    use kiapi::board::types::dimension::DimensionStyle;

    if let Some(text) = &mut dimension.text {
        translate_text(text, dx_nm, dy_nm);
    }
    match dimension.dimension_style.as_mut() {
        Some(DimensionStyle::Aligned(style)) => {
            translate_vector(&mut style.start, dx_nm, dy_nm);
            translate_vector(&mut style.end, dx_nm, dy_nm);
        }
        Some(DimensionStyle::Orthogonal(style)) => {
            translate_vector(&mut style.start, dx_nm, dy_nm);
            translate_vector(&mut style.end, dx_nm, dy_nm);
        }
        Some(DimensionStyle::Radial(style)) => {
            translate_vector(&mut style.center, dx_nm, dy_nm);
            translate_vector(&mut style.radius_point, dx_nm, dy_nm);
        }
        Some(DimensionStyle::Leader(style)) => {
            translate_vector(&mut style.start, dx_nm, dy_nm);
            translate_vector(&mut style.end, dx_nm, dy_nm);
        }
        Some(DimensionStyle::Center(style)) => {
            translate_vector(&mut style.center, dx_nm, dy_nm);
            translate_vector(&mut style.end, dx_nm, dy_nm);
        }
        None => {}
    }
}

#[derive(Clone, Copy)]
struct Rotation {
    center_x_nm: i64,
    center_y_nm: i64,
    sin: f64,
    cos: f64,
    delta_degrees: f64,
}

impl Rotation {
    fn new(center_x_nm: i64, center_y_nm: i64, delta_degrees: f64) -> Self {
        let radians = delta_degrees.to_radians();
        Self {
            center_x_nm,
            center_y_nm,
            sin: radians.sin(),
            cos: radians.cos(),
            delta_degrees,
        }
    }

    fn vector(self, vector: &mut Option<kiapi::common::types::Vector2>) {
        let Some(vector) = vector else {
            return;
        };
        let x = (vector.x_nm - self.center_x_nm) as f64;
        let y = (vector.y_nm - self.center_y_nm) as f64;
        // KiCad's positive board angles are clockwise in its Y-down board
        // coordinate system.
        vector.x_nm = self.center_x_nm + (x * self.cos + y * self.sin).round() as i64;
        vector.y_nm = self.center_y_nm + (-x * self.sin + y * self.cos).round() as i64;
    }

    fn angle(self, angle: &mut Option<kiapi::common::types::Angle>) {
        if let Some(angle) = angle {
            angle.value_degrees += self.delta_degrees;
        }
    }
}

fn rotate_poly_line(line: &mut kiapi::common::types::PolyLine, rotation: Rotation) {
    use kiapi::common::types::poly_line_node::Geometry;

    for node in &mut line.nodes {
        match node.geometry.as_mut() {
            Some(Geometry::Point(point)) => {
                let x = (point.x_nm - rotation.center_x_nm) as f64;
                let y = (point.y_nm - rotation.center_y_nm) as f64;
                point.x_nm =
                    rotation.center_x_nm + (x * rotation.cos + y * rotation.sin).round() as i64;
                point.y_nm =
                    rotation.center_y_nm + (-x * rotation.sin + y * rotation.cos).round() as i64;
            }
            Some(Geometry::Arc(arc)) => {
                rotation.vector(&mut arc.start);
                rotation.vector(&mut arc.mid);
                rotation.vector(&mut arc.end);
            }
            None => {}
        }
    }
}

fn rotate_poly_set(set: &mut Option<kiapi::common::types::PolySet>, rotation: Rotation) {
    if let Some(set) = set {
        rotate_poly_set_value(set, rotation);
    }
}

fn rotate_poly_set_value(set: &mut kiapi::common::types::PolySet, rotation: Rotation) {
    for polygon in &mut set.polygons {
        if let Some(outline) = &mut polygon.outline {
            rotate_poly_line(outline, rotation);
        }
        for hole in &mut polygon.holes {
            rotate_poly_line(hole, rotation);
        }
    }
}

fn rotate_text(text: &mut kiapi::common::types::Text, rotation: Rotation) {
    rotation.vector(&mut text.position);
    if let Some(attributes) = &mut text.attributes {
        rotation.angle(&mut attributes.angle);
    }
}

fn rotate_field(field: &mut kiapi::board::types::Field, rotation: Rotation) {
    if let Some(text) = field
        .text
        .as_mut()
        .and_then(|board_text| board_text.text.as_mut())
    {
        rotate_text(text, rotation);
    }
}

fn rotate_graphic_shape(shape: &mut kiapi::common::types::GraphicShape, rotation: Rotation) {
    use kiapi::common::types::graphic_shape::Geometry;

    match shape.geometry.as_mut() {
        Some(Geometry::Segment(segment)) => {
            rotation.vector(&mut segment.start);
            rotation.vector(&mut segment.end);
        }
        Some(Geometry::Rectangle(rectangle)) => {
            rotation.vector(&mut rectangle.top_left);
            rotation.vector(&mut rectangle.bottom_right);
        }
        Some(Geometry::Arc(arc)) => {
            rotation.vector(&mut arc.start);
            rotation.vector(&mut arc.mid);
            rotation.vector(&mut arc.end);
        }
        Some(Geometry::Circle(circle)) => {
            rotation.vector(&mut circle.center);
            rotation.vector(&mut circle.radius_point);
        }
        Some(Geometry::Polygon(polygon)) => rotate_poly_set_value(polygon, rotation),
        Some(Geometry::Bezier(bezier)) => {
            rotation.vector(&mut bezier.start);
            rotation.vector(&mut bezier.control1);
            rotation.vector(&mut bezier.control2);
            rotation.vector(&mut bezier.end);
        }
        None => {}
    }
}

fn rotate_dimension(dimension: &mut kiapi::board::types::Dimension, rotation: Rotation) {
    use kiapi::board::types::dimension::DimensionStyle;

    if let Some(text) = &mut dimension.text {
        rotate_text(text, rotation);
    }
    match dimension.dimension_style.as_mut() {
        Some(DimensionStyle::Aligned(style)) => {
            rotation.vector(&mut style.start);
            rotation.vector(&mut style.end);
        }
        Some(DimensionStyle::Orthogonal(style)) => {
            rotation.vector(&mut style.start);
            rotation.vector(&mut style.end);
        }
        Some(DimensionStyle::Radial(style)) => {
            rotation.vector(&mut style.center);
            rotation.vector(&mut style.radius_point);
        }
        Some(DimensionStyle::Leader(style)) => {
            rotation.vector(&mut style.start);
            rotation.vector(&mut style.end);
        }
        Some(DimensionStyle::Center(style)) => {
            rotation.vector(&mut style.center);
            rotation.vector(&mut style.end);
        }
        None => {}
    }
}

fn repack_any<M: Message>(any: &mut prost_types::Any, message: &M) {
    any.value = message.encode_to_vec();
}

/// KiCad 10 serializes footprint children using board-space coordinates even
/// though several protobuf comments call them footprint-relative. Updating a
/// footprint replaces the whole object, so its children must be translated by
/// the same delta as the parent or they remain physically behind and KiCad
/// persists corrupt relative offsets (mixelpixx/Konnect#23).
fn translate_footprint_child(any: &mut prost_types::Any, dx_nm: i64, dy_nm: i64) -> Result<()> {
    match any.type_url.as_str() {
        "type.googleapis.com/kiapi.board.types.Pad" => {
            let mut item: kiapi::board::types::Pad = unpack_any(any)?;
            translate_vector(&mut item.position, dx_nm, dy_nm);
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Field" => {
            let mut item: kiapi::board::types::Field = unpack_any(any)?;
            translate_field(&mut item, dx_nm, dy_nm);
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardText" => {
            let mut item: kiapi::board::types::BoardText = unpack_any(any)?;
            if let Some(text) = &mut item.text {
                translate_text(text, dx_nm, dy_nm);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardTextBox" => {
            let mut item: kiapi::board::types::BoardTextBox = unpack_any(any)?;
            if let Some(textbox) = &mut item.textbox {
                translate_vector(&mut textbox.top_left, dx_nm, dy_nm);
                translate_vector(&mut textbox.bottom_right, dx_nm, dy_nm);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardGraphicShape" => {
            let mut item: kiapi::board::types::BoardGraphicShape = unpack_any(any)?;
            if let Some(shape) = &mut item.shape {
                translate_graphic_shape(shape, dx_nm, dy_nm);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Zone" => {
            let mut item: kiapi::board::types::Zone = unpack_any(any)?;
            translate_poly_set(&mut item.outline, dx_nm, dy_nm);
            for filled in &mut item.filled_polygons {
                translate_poly_set(&mut filled.shapes, dx_nm, dy_nm);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Dimension" => {
            let mut item: kiapi::board::types::Dimension = unpack_any(any)?;
            translate_dimension(&mut item, dx_nm, dy_nm);
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Group"
        | "type.googleapis.com/kiapi.board.types.Footprint3DModel" => {}
        unsupported => anyhow::bail!(
            "cannot safely move footprint containing unsupported KiCad child type '{unsupported}'"
        ),
    }
    Ok(())
}

fn rotate_footprint_child(any: &mut prost_types::Any, rotation: Rotation) -> Result<()> {
    match any.type_url.as_str() {
        "type.googleapis.com/kiapi.board.types.Pad" => {
            let mut item: kiapi::board::types::Pad = unpack_any(any)?;
            rotation.vector(&mut item.position);
            if let Some(stack) = &mut item.pad_stack {
                rotation.angle(&mut stack.angle);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Field" => {
            let mut item: kiapi::board::types::Field = unpack_any(any)?;
            rotate_field(&mut item, rotation);
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardText" => {
            let mut item: kiapi::board::types::BoardText = unpack_any(any)?;
            if let Some(text) = &mut item.text {
                rotate_text(text, rotation);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardTextBox" => {
            let mut item: kiapi::board::types::BoardTextBox = unpack_any(any)?;
            if let Some(textbox) = &mut item.textbox {
                rotation.vector(&mut textbox.top_left);
                rotation.vector(&mut textbox.bottom_right);
                if let Some(attributes) = &mut textbox.attributes {
                    rotation.angle(&mut attributes.angle);
                }
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.BoardGraphicShape" => {
            let mut item: kiapi::board::types::BoardGraphicShape = unpack_any(any)?;
            if let Some(shape) = &mut item.shape {
                rotate_graphic_shape(shape, rotation);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Zone" => {
            let mut item: kiapi::board::types::Zone = unpack_any(any)?;
            rotate_poly_set(&mut item.outline, rotation);
            for filled in &mut item.filled_polygons {
                rotate_poly_set(&mut filled.shapes, rotation);
            }
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Dimension" => {
            let mut item: kiapi::board::types::Dimension = unpack_any(any)?;
            rotate_dimension(&mut item, rotation);
            repack_any(any, &item);
        }
        "type.googleapis.com/kiapi.board.types.Group"
        | "type.googleapis.com/kiapi.board.types.Footprint3DModel" => {}
        unsupported => anyhow::bail!(
            "cannot safely rotate footprint containing unsupported KiCad child type '{unsupported}'"
        ),
    }
    Ok(())
}

pub struct KiCadIpcClient {
    socket_path: String,
    kicad_token: String,
    client_name: String,
}

impl KiCadIpcClient {
    /// Create a client connecting to the given IPC socket path.
    /// If empty, tries KICAD_API_SOCKET environment variable.
    pub fn new(socket_path: impl Into<String>) -> Self {
        let path = socket_path.into();
        let effective_path = if path.is_empty() {
            std::env::var("KICAD_API_SOCKET").unwrap_or_default()
        } else {
            path
        };
        KiCadIpcClient {
            socket_path: effective_path,
            kicad_token: std::env::var("KICAD_API_TOKEN").unwrap_or_default(),
            client_name: format!("konnect-{}", std::process::id()),
        }
    }

    /// Create a client with an explicit API token.
    ///
    /// KiCAD supplies this token to executable plugins through
    /// `KICAD_API_TOKEN`. This constructor is useful for clients that obtain
    /// the connection details through another discovery mechanism.
    pub fn new_with_token(socket_path: impl Into<String>, token: impl Into<String>) -> Self {
        let mut client = Self::new(socket_path);
        client.kicad_token = token.into();
        client
    }

    /// Send a protobuf command and return the response Any.
    fn send_command(
        &self,
        command: &impl Message,
        type_name: &str,
    ) -> Result<Option<prost_types::Any>> {
        if self.socket_path.is_empty() {
            anyhow::bail!(
                "KiCAD IPC socket path not configured. To fix: \
                 (1) in KiCAD, enable Edit > Preferences > Plugins > 'Enable KiCad API' \
                 and copy the listed ipc:// address; \
                 (2) paste it into the 'IPC Socket' field of the Konnect settings dialog \
                 (Tools > External Plugins > Konnect) and save; \
                 (3) restart the AI client so the server rereads settings. \
                 Alternatively set ipc_socket_path in konnect-settings.json or launch \
                 via KiCAD (which sets KICAD_API_SOCKET). \
                 Full guide: https://github.com/perara/Konnect/blob/main/docs/TROUBLESHOOTING.md"
            );
        }

        let request = kiapi::common::ApiRequest {
            header: Some(kiapi::common::ApiRequestHeader {
                kicad_token: self.kicad_token.clone(),
                client_name: self.client_name.clone(),
            }),
            message: Some(pack_any(command, type_name)),
        };

        let request_bytes = request.encode_to_vec();
        debug!(
            "[BETA] IPC → {} ({} bytes) to {}",
            type_name,
            request_bytes.len(),
            self.socket_path
        );

        // Connect via NNG req0 socket
        let socket =
            nng::Socket::new(nng::Protocol::Req0).context("Failed to create NNG socket")?;

        // Bound every step: a busy or wedged KiCAD must produce an error the
        // tools can surface, never an indefinite hang (the predecessor
        // project's sync/autoroute hangs blocked for >600 s on exactly this).
        // 30 s receive allows slow board operations like zone refills.
        use nng::options::Options;
        socket
            .set_opt::<nng::options::SendTimeout>(Some(std::time::Duration::from_secs(5)))
            .context("Failed to set NNG send timeout")?;
        socket
            .set_opt::<nng::options::RecvTimeout>(Some(std::time::Duration::from_secs(30)))
            .context("Failed to set NNG receive timeout")?;

        // Build the dial URL
        let dial_url =
            if self.socket_path.starts_with("ipc://") || self.socket_path.starts_with("tcp://") {
                self.socket_path.clone()
            } else {
                format!("ipc://{}", self.socket_path)
            };

        socket
            .dial(&dial_url)
            .with_context(|| format!("Cannot connect to KiCAD IPC at {}", dial_url))?;

        // Send request
        let msg = nng::Message::from(request_bytes.as_slice());
        socket
            .send(msg)
            .map_err(|(_, e)| anyhow::anyhow!("NNG send failed: {}", e))?;

        // Receive response
        let reply = socket
            .recv()
            .map_err(|e| anyhow::anyhow!("NNG recv failed: {}", e))?;

        let response = kiapi::common::ApiResponse::decode(reply.as_slice())
            .context("Failed to decode ApiResponse")?;

        // Check status
        if let Some(ref status) = response.status {
            let code = status.status();
            if code != kiapi::common::ApiStatusCode::AsOk {
                let msg = if status.error_message.is_empty() {
                    format!("{:?}", code)
                } else {
                    status.error_message.clone()
                };
                debug!("[BETA] IPC ← error: {} ({})", msg, code.as_str_name());
                anyhow::bail!("KiCAD IPC error: {} ({})", msg, code.as_str_name());
            }
        }

        debug!("[BETA] IPC ← OK");
        Ok(response.message)
    }

    // ─── Public API (same interface as before, tools don't change) ───────

    /// Check if KiCAD is reachable.
    pub fn ping(&self) -> Result<bool> {
        let ping = kiapi::common::commands::Ping {};
        match self.send_command(&ping, "kiapi.common.commands.Ping") {
            Ok(_) => Ok(true),
            Err(e) => {
                warn!("[BETA] Ping failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Get the list of open documents (boards).
    pub fn get_open_documents(&self) -> Result<Vec<kiapi::common::types::DocumentSpecifier>> {
        let cmd = kiapi::common::commands::GetOpenDocuments {
            r#type: kiapi::common::types::DocumentType::DoctypePcb as i32,
        };
        let response_any = self.send_command(&cmd, "kiapi.common.commands.GetOpenDocuments")?;
        if let Some(any) = response_any {
            let resp: kiapi::common::commands::GetOpenDocumentsResponse = unpack_any(&any)?;
            Ok(resp.documents)
        } else {
            Ok(vec![])
        }
    }

    /// Get the first open PCB's DocumentSpecifier (needed for most commands).
    fn get_board_document(&self) -> Result<kiapi::common::types::DocumentSpecifier> {
        let docs = self.get_open_documents()?;
        docs.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("No PCB document is open in KiCAD. Open a board file first.")
        })
    }

    fn make_header(&self) -> Result<kiapi::common::types::ItemHeader> {
        Ok(kiapi::common::types::ItemHeader {
            document: Some(self.get_board_document()?),
            container: None,
            field_mask: None,
        })
    }

    /// Get all nets on the board.
    pub fn get_nets(&self) -> Result<Vec<IpcNet>> {
        let doc = self.get_board_document()?;
        let cmd = kiapi::board::commands::GetNets {
            board: Some(doc),
            netclass_filter: vec![],
        };
        let response_any = self.send_command(&cmd, "kiapi.board.commands.GetNets")?;
        if let Some(any) = response_any {
            let resp: kiapi::board::commands::NetsResponse = unpack_any(&any)?;
            Ok(resp
                .nets
                .iter()
                .map(|n| IpcNet {
                    name: n.name.clone(),
                    netcode: n.code.as_ref().map(|c| c.value).unwrap_or(0),
                })
                .collect())
        } else {
            Ok(vec![])
        }
    }

    /// Get board items by type.
    pub fn get_items(
        &self,
        item_type: kiapi::common::types::KiCadObjectType,
    ) -> Result<Vec<prost_types::Any>> {
        let header = self.make_header()?;
        let cmd = kiapi::common::commands::GetItems {
            header: Some(header),
            types: vec![item_type as i32],
        };
        let response_any = self.send_command(&cmd, "kiapi.common.commands.GetItems")?;
        if let Some(any) = response_any {
            let resp: kiapi::common::commands::GetItemsResponse = unpack_any(&any)?;
            Ok(resp.items)
        } else {
            Ok(vec![])
        }
    }

    /// List all footprints on the board.
    pub fn list_footprints(&self) -> Result<Vec<IpcFootprint>> {
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbFootprint)?;
        let mut footprints = Vec::new();
        for item in &items {
            if let Ok(fp) = kiapi::board::types::FootprintInstance::decode(item.value.as_slice()) {
                let pos = fp.position.as_ref();
                let ref_text = fp
                    .reference_field
                    .as_ref()
                    .and_then(|f| f.text.as_ref())
                    .and_then(|bt| bt.text.as_ref())
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let val_text = fp
                    .value_field
                    .as_ref()
                    .and_then(|f| f.text.as_ref())
                    .and_then(|bt| bt.text.as_ref())
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                let lib_id = fp
                    .definition
                    .as_ref()
                    .and_then(|d| d.id.as_ref())
                    .map(|id| format!("{}:{}", id.library_nickname, id.entry_name))
                    .unwrap_or_default();
                footprints.push(IpcFootprint {
                    reference: ref_text,
                    value: val_text,
                    footprint: lib_id,
                    position: IpcVector2 {
                        x: pos.map(|p| nm_to_mm(p.x_nm)).unwrap_or(0.0),
                        y: pos.map(|p| nm_to_mm(p.y_nm)).unwrap_or(0.0),
                    },
                    rotation: fp
                        .orientation
                        .as_ref()
                        .map(|a| a.value_degrees)
                        .unwrap_or(0.0),
                    layer: layer_enum_to_name(fp.layer).to_string(),
                });
            }
        }
        Ok(footprints)
    }

    /// Create items on the board.
    pub fn create_items(&self, items: Vec<prost_types::Any>) -> Result<()> {
        let header = self.make_header()?;
        let cmd = kiapi::common::commands::CreateItems {
            header: Some(header),
            items,
            container: None,
        };
        if let Some(any) = self.send_command(&cmd, "kiapi.common.commands.CreateItems")? {
            let response: kiapi::common::commands::CreateItemsResponse = unpack_any(&any)?;
            for result in response.created_items {
                let status = result
                    .status
                    .context("KiCAD returned a creation result without item status")?;
                if status.code() != kiapi::common::commands::ItemStatusCode::IscOk {
                    anyhow::bail!(
                        "KiCAD item creation failed: {} ({})",
                        status.error_message,
                        status.code().as_str_name()
                    );
                }
            }
        }
        Ok(())
    }

    /// Update existing items by KIID. Generic wrapper mirroring create_items/delete_items;
    /// each `Any` must be a fully-formed board item with an existing `id` populated.
    pub fn update_items(&self, items: Vec<prost_types::Any>) -> Result<()> {
        let header = self.make_header()?;
        let cmd = kiapi::common::commands::UpdateItems {
            header: Some(header),
            items,
        };
        if let Some(any) = self.send_command(&cmd, "kiapi.common.commands.UpdateItems")? {
            let response: kiapi::common::commands::UpdateItemsResponse = unpack_any(&any)?;
            for result in response.updated_items {
                let Some(status) = result.status else {
                    anyhow::bail!("KiCAD returned an update result without item status");
                };
                if status.code() != kiapi::common::commands::ItemStatusCode::IscOk {
                    anyhow::bail!(
                        "KiCAD item update failed: {} ({})",
                        status.error_message,
                        status.code().as_str_name()
                    );
                }
            }
        }
        Ok(())
    }

    /// Delete items by KIID.
    pub fn delete_items(&self, ids: Vec<String>) -> Result<()> {
        let header = self.make_header()?;
        let cmd = kiapi::common::commands::DeleteItems {
            header: Some(header),
            item_ids: ids
                .iter()
                .map(|id| kiapi::common::types::Kiid { value: id.clone() })
                .collect(),
        };
        self.send_command(&cmd, "kiapi.common.commands.DeleteItems")?;
        Ok(())
    }

    /// Refill zones on the board.
    pub fn refill_zones(&self) -> Result<()> {
        let doc = self.get_board_document()?;
        let cmd = kiapi::board::commands::RefillZones {
            board: Some(doc),
            zones: vec![],
        };
        self.send_command(&cmd, "kiapi.board.commands.RefillZones")?;
        Ok(())
    }

    /// Save the open board document.
    pub fn save_board(&self) -> Result<()> {
        let doc = self.get_board_document()?;
        let cmd = kiapi::common::commands::SaveDocument {
            document: Some(doc),
        };
        self.send_command(&cmd, "kiapi.common.commands.SaveDocument")?;
        Ok(())
    }

    /// Begin a commit (undo group).
    pub fn begin_commit(&self) -> Result<String> {
        let cmd = kiapi::common::commands::BeginCommit {};
        let response_any = self.send_command(&cmd, "kiapi.common.commands.BeginCommit")?;
        if let Some(any) = response_any {
            let resp: kiapi::common::commands::BeginCommitResponse = unpack_any(&any)?;
            Ok(resp.id.map(|id| id.value).unwrap_or_default())
        } else {
            Ok(String::new())
        }
    }

    /// End a commit (push or drop).
    pub fn end_commit(
        &self,
        commit_id: &str,
        action: kiapi::common::commands::CommitAction,
        message: &str,
    ) -> Result<()> {
        let cmd = kiapi::common::commands::EndCommit {
            id: Some(kiapi::common::types::Kiid {
                value: commit_id.to_string(),
            }),
            action: action as i32,
            message: message.to_string(),
        };
        self.send_command(&cmd, "kiapi.common.commands.EndCommit")?;
        Ok(())
    }

    /// Push (commit) changes.
    pub fn push_commit(&self, commit_id: &str, description: &str) -> Result<()> {
        self.end_commit(
            commit_id,
            kiapi::common::commands::CommitAction::CmaCommit,
            description,
        )
    }

    /// Drop (rollback) changes.
    pub fn drop_commit(&self, commit_id: &str) -> Result<()> {
        self.end_commit(
            commit_id,
            kiapi::common::commands::CommitAction::CmaDrop,
            "",
        )
    }

    // ─── PCB Item Operations (real protobuf implementations) ───────────

    /// Resolve a net name to its net code by querying GetNets.
    pub fn resolve_net_code(&self, net_name: &str) -> Result<i32> {
        let nets = self.get_nets()?;
        nets.iter()
            .find(|n| n.name == net_name)
            .map(|n| n.netcode)
            .ok_or_else(|| anyhow::anyhow!("Net '{}' not found on board", net_name))
    }

    /// Find a footprint by reference and return its IpcFootprint + KIID.
    pub fn get_footprint(&self, reference: &str) -> Result<Option<IpcFootprint>> {
        let footprints = self.list_footprints()?;
        Ok(footprints.into_iter().find(|fp| fp.reference == reference))
    }

    /// Find a footprint's KIID by reference.
    fn find_footprint_kiid(&self, reference: &str) -> Result<String> {
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbFootprint)?;
        for item in &items {
            if let Ok(fp) = kiapi::board::types::FootprintInstance::decode(item.value.as_slice()) {
                let ref_text = fp
                    .reference_field
                    .as_ref()
                    .and_then(|f| f.text.as_ref())
                    .and_then(|bt| bt.text.as_ref())
                    .map(|t| t.text.as_str())
                    .unwrap_or("");
                if ref_text == reference {
                    if let Some(id) = &fp.id {
                        return Ok(id.value.clone());
                    }
                }
            }
        }
        anyhow::bail!("Footprint '{}' not found on board", reference)
    }

    /// Add a track segment to the board.
    #[allow(clippy::too_many_arguments)]
    pub fn add_track(
        &self,
        net_name: &str,
        layer: &str,
        width: f64,
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
    ) -> Result<()> {
        let net_code = self.resolve_net_code(net_name)?;
        let track = crate::builders::build_track(net_name, net_code, layer, width, x1, y1, x2, y2);
        let any = crate::builders::pack_any(&track, "kiapi.board.types.Track");
        self.create_items(vec![any])?;
        Ok(())
    }

    /// Add a via to the board using S-expression string (simpler than full protobuf PadStack construction).
    pub fn add_via(&self, net_name: &str, x: f64, y: f64, drill: f64, pad_size: f64) -> Result<()> {
        let net_code = self.resolve_net_code(net_name)?;
        let sexp = crate::builders::via_sexp(net_name, net_code, x, y, drill, pad_size);
        let doc = self.get_board_document()?;
        let cmd = kiapi::common::commands::ParseAndCreateItemsFromString {
            document: Some(doc),
            contents: sexp,
        };
        self.send_command(&cmd, "kiapi.common.commands.ParseAndCreateItemsFromString")?;
        Ok(())
    }

    /// Delete a track by UUID.
    pub fn delete_track(&self, uuid: &str) -> Result<()> {
        self.delete_items(vec![uuid.to_string()])
    }

    /// Query tracks, optionally filtered by net and/or layer.
    pub fn get_tracks(
        &self,
        net_filter: Option<&str>,
        layer_filter: Option<&str>,
    ) -> Result<Vec<IpcTrack>> {
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbTrace)?;
        let mut tracks = Vec::new();
        for item in &items {
            if let Ok(track) = kiapi::board::types::Track::decode(item.value.as_slice()) {
                let net_name = track.net.as_ref().map(|n| n.name.as_str()).unwrap_or("");
                let layer_name = layer_enum_to_name(track.layer);

                // Apply net filter
                if let Some(nf) = net_filter {
                    if net_name != nf {
                        continue;
                    }
                }
                // Apply layer filter
                if let Some(lf) = layer_filter {
                    if layer_name != lf {
                        continue;
                    }
                }

                let start = track.start.as_ref();
                let end = track.end.as_ref();
                tracks.push(IpcTrack {
                    net_name: net_name.to_string(),
                    layer: layer_name.to_string(),
                    width: track
                        .width
                        .as_ref()
                        .map(|w| crate::builders::nm_to_mm(w.value_nm))
                        .unwrap_or(0.25),
                    start: IpcVector2 {
                        x: start
                            .map(|p| crate::builders::nm_to_mm(p.x_nm))
                            .unwrap_or(0.0),
                        y: start
                            .map(|p| crate::builders::nm_to_mm(p.y_nm))
                            .unwrap_or(0.0),
                    },
                    end: IpcVector2 {
                        x: end
                            .map(|p| crate::builders::nm_to_mm(p.x_nm))
                            .unwrap_or(0.0),
                        y: end
                            .map(|p| crate::builders::nm_to_mm(p.y_nm))
                            .unwrap_or(0.0),
                    },
                });
            }
        }
        Ok(tracks)
    }

    /// Move a footprint to a new position.
    pub fn move_footprint(&self, reference: &str, x: f64, y: f64) -> Result<()> {
        // Find the footprint, update position, send UpdateItems
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbFootprint)?;
        for item in &items {
            if let Ok(mut fp) =
                kiapi::board::types::FootprintInstance::decode(item.value.as_slice())
            {
                let ref_text = fp
                    .reference_field
                    .as_ref()
                    .and_then(|f| f.text.as_ref())
                    .and_then(|bt| bt.text.as_ref())
                    .map(|t| t.text.as_str())
                    .unwrap_or("");
                if ref_text == reference {
                    let old_position = fp.position.as_ref().context(
                        "KiCAD returned a footprint without a position; refusing unsafe move",
                    )?;
                    let new_position = crate::builders::vec2(x, y);
                    let dx_nm = new_position.x_nm.saturating_sub(old_position.x_nm);
                    let dy_nm = new_position.y_nm.saturating_sub(old_position.y_nm);

                    if let Some(field) = &mut fp.reference_field {
                        translate_field(field, dx_nm, dy_nm);
                    }
                    if let Some(field) = &mut fp.value_field {
                        translate_field(field, dx_nm, dy_nm);
                    }
                    if let Some(field) = &mut fp.datasheet_field {
                        translate_field(field, dx_nm, dy_nm);
                    }
                    if let Some(field) = &mut fp.description_field {
                        translate_field(field, dx_nm, dy_nm);
                    }
                    if let Some(definition) = &mut fp.definition {
                        for child in &mut definition.items {
                            translate_footprint_child(child, dx_nm, dy_nm)?;
                        }
                    }
                    fp.position = Some(new_position);
                    let any = crate::builders::pack_any(&fp, "kiapi.board.types.FootprintInstance");
                    self.update_items(vec![any])?;
                    return Ok(());
                }
            }
        }
        anyhow::bail!("Footprint '{}' not found", reference)
    }

    /// Rotate a footprint to a new angle.
    pub fn rotate_footprint(&self, reference: &str, angle: f64) -> Result<()> {
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbFootprint)?;
        for item in &items {
            if let Ok(mut fp) =
                kiapi::board::types::FootprintInstance::decode(item.value.as_slice())
            {
                let ref_text = fp
                    .reference_field
                    .as_ref()
                    .and_then(|f| f.text.as_ref())
                    .and_then(|bt| bt.text.as_ref())
                    .map(|t| t.text.as_str())
                    .unwrap_or("");
                if ref_text == reference {
                    let position = fp.position.as_ref().context(
                        "KiCAD returned a footprint without a position; refusing unsafe rotation",
                    )?;
                    let old_angle = fp
                        .orientation
                        .as_ref()
                        .map(|orientation| orientation.value_degrees)
                        .unwrap_or(0.0);
                    let rotation = Rotation::new(position.x_nm, position.y_nm, angle - old_angle);

                    if let Some(field) = &mut fp.reference_field {
                        rotate_field(field, rotation);
                    }
                    if let Some(field) = &mut fp.value_field {
                        rotate_field(field, rotation);
                    }
                    if let Some(field) = &mut fp.datasheet_field {
                        rotate_field(field, rotation);
                    }
                    if let Some(field) = &mut fp.description_field {
                        rotate_field(field, rotation);
                    }
                    if let Some(definition) = &mut fp.definition {
                        for child in &mut definition.items {
                            rotate_footprint_child(child, rotation)?;
                        }
                    }
                    fp.orientation = Some(kiapi::common::types::Angle {
                        value_degrees: angle,
                    });
                    let any = crate::builders::pack_any(&fp, "kiapi.board.types.FootprintInstance");
                    self.update_items(vec![any])?;
                    return Ok(());
                }
            }
        }
        anyhow::bail!("Footprint '{}' not found", reference)
    }

    /// Update the visible value field of an existing footprint.
    pub fn set_footprint_value(&self, reference: &str, value: &str) -> Result<()> {
        let items = self.get_items(kiapi::common::types::KiCadObjectType::KotPcbFootprint)?;
        for item in items {
            if let Ok(mut footprint) =
                kiapi::board::types::FootprintInstance::decode(item.value.as_slice())
            {
                let current_reference = footprint
                    .reference_field
                    .as_ref()
                    .and_then(|field| field.text.as_ref())
                    .and_then(|board_text| board_text.text.as_ref())
                    .map(|text| text.text.as_str())
                    .unwrap_or("");
                if current_reference != reference {
                    continue;
                }
                if let Some(text) = footprint
                    .value_field
                    .as_mut()
                    .and_then(|field| field.text.as_mut())
                    .and_then(|board_text| board_text.text.as_mut())
                {
                    text.text = value.to_string();
                } else {
                    anyhow::bail!("Footprint '{reference}' has no editable value field");
                }
                if let Some(text) = footprint
                    .definition
                    .as_mut()
                    .and_then(|definition| definition.value_field.as_mut())
                    .and_then(|field| field.text.as_mut())
                    .and_then(|board_text| board_text.text.as_mut())
                {
                    text.text = value.to_string();
                }
                let any =
                    crate::builders::pack_any(&footprint, "kiapi.board.types.FootprintInstance");
                self.update_items(vec![any])?;
                return Ok(());
            }
        }
        anyhow::bail!("Footprint '{reference}' not found")
    }

    /// Delete a footprint by reference.
    pub fn delete_footprint(&self, reference: &str) -> Result<()> {
        let kiid = self.find_footprint_kiid(reference)?;
        self.delete_items(vec![kiid])
    }

    /// Place a footprint instance through the typed KiCAD CreateItems API.
    #[allow(clippy::too_many_arguments)]
    pub fn place_footprint(
        &self,
        lib_id: &str,
        reference: &str,
        value: &str,
        pads: &[IpcPadDefinition],
        x: f64,
        y: f64,
        rotation: f64,
        layer: &str,
    ) -> Result<IpcFootprint> {
        let (library_nickname, entry_name) = lib_id
            .split_once(':')
            .context("footprint identifier must use Library:Footprint syntax")?;
        let text_field =
            |name: &str, text: &str, field_y: f64, visible: bool| kiapi::board::types::Field {
                id: None,
                name: name.to_string(),
                text: Some(kiapi::board::types::BoardText {
                    id: None,
                    text: Some(kiapi::common::types::Text {
                        position: Some(crate::builders::vec2(x, field_y)),
                        attributes: Some(kiapi::common::types::TextAttributes {
                            size: Some(crate::builders::vec2(1.0, 1.0)),
                            angle: Some(kiapi::common::types::Angle {
                                value_degrees: rotation,
                            }),
                            ..Default::default()
                        }),
                        text: text.to_string(),
                        hyperlink: String::new(),
                    }),
                    layer: crate::builders::layer_from_name(if layer == "B.Cu" {
                        "B.SilkS"
                    } else {
                        "F.SilkS"
                    }) as i32,
                    knockout: false,
                    locked: kiapi::common::types::LockedState::LsUnlocked as i32,
                }),
                visible,
            };
        let reference_field = text_field("Reference", reference, y - 1.0, true);
        let value_field = text_field("Value", value, y + 1.0, false);
        let radians = rotation.to_radians();
        let child_items = pads
            .iter()
            .map(|pad| {
                let local_x = pad.x;
                let local_y = pad.y;
                let board_x = x + local_x * radians.cos() + local_y * radians.sin();
                let board_y = y - local_x * radians.sin() + local_y * radians.cos();
                let mut layers = Vec::new();
                for name in &pad.layers {
                    match name.as_str() {
                        "*.Cu" => layers.extend(3..=34),
                        "*.Mask" => layers.extend([
                            kiapi::board::types::BoardLayer::BlFMask as i32,
                            kiapi::board::types::BoardLayer::BlBMask as i32,
                        ]),
                        "*.Paste" => layers.extend([
                            kiapi::board::types::BoardLayer::BlFPaste as i32,
                            kiapi::board::types::BoardLayer::BlBPaste as i32,
                        ]),
                        name => layers.push(crate::builders::layer_from_name(name) as i32),
                    }
                }
                layers
                    .retain(|layer| *layer != kiapi::board::types::BoardLayer::BlUndefined as i32);
                layers.sort_unstable();
                layers.dedup();

                let shape = match pad.shape.as_str() {
                    "circle" => kiapi::board::types::PadStackShape::PssCircle,
                    "rect" => kiapi::board::types::PadStackShape::PssRectangle,
                    "oval" => kiapi::board::types::PadStackShape::PssOval,
                    "trapezoid" => kiapi::board::types::PadStackShape::PssTrapezoid,
                    "roundrect" => kiapi::board::types::PadStackShape::PssRoundrect,
                    "chamfered_rect" => kiapi::board::types::PadStackShape::PssChamferedrect,
                    _ => kiapi::board::types::PadStackShape::PssRectangle,
                };
                let copper_layer =
                    if layers.contains(&(kiapi::board::types::BoardLayer::BlFCu as i32)) {
                        kiapi::board::types::BoardLayer::BlFCu
                    } else {
                        kiapi::board::types::BoardLayer::BlBCu
                    };
                let copper = kiapi::board::types::PadStackLayer {
                    layer: copper_layer as i32,
                    shape: shape as i32,
                    size: Some(crate::builders::vec2(pad.size_x, pad.size_y)),
                    corner_rounding_ratio: pad.roundrect_ratio,
                    custom_anchor_shape: shape as i32,
                    offset: Some(crate::builders::vec2(0.0, 0.0)),
                    ..Default::default()
                };
                let drill = pad
                    .drill_x
                    .map(|drill_x| kiapi::board::types::DrillProperties {
                        start_layer: kiapi::board::types::BoardLayer::BlFCu as i32,
                        end_layer: kiapi::board::types::BoardLayer::BlBCu as i32,
                        diameter: Some(crate::builders::vec2(
                            drill_x,
                            pad.drill_y.unwrap_or(drill_x),
                        )),
                        shape: if pad.drill_oval {
                            kiapi::board::types::DrillShape::DsOblong as i32
                        } else {
                            kiapi::board::types::DrillShape::DsCircle as i32
                        },
                        ..Default::default()
                    });
                let stack = kiapi::board::types::PadStack {
                    r#type: kiapi::board::types::PadStackType::PstNormal as i32,
                    layers,
                    drill,
                    unconnected_layer_removal: kiapi::board::types::UnconnectedLayerRemoval::UlrKeep
                        as i32,
                    copper_layers: vec![copper],
                    angle: Some(kiapi::common::types::Angle {
                        value_degrees: rotation + pad.rotation,
                    }),
                    ..Default::default()
                };
                let pad_type = match pad.pad_type.as_str() {
                    "thru_hole" => kiapi::board::types::PadType::PtPth,
                    "np_thru_hole" => kiapi::board::types::PadType::PtNpth,
                    "connect" => kiapi::board::types::PadType::PtEdgeConnector,
                    _ => kiapi::board::types::PadType::PtSmd,
                };
                let item = kiapi::board::types::Pad {
                    number: pad.number.clone(),
                    r#type: pad_type as i32,
                    pad_stack: Some(stack),
                    position: Some(crate::builders::vec2(board_x, board_y)),
                    locked: kiapi::common::types::LockedState::LsUnlocked as i32,
                    ..Default::default()
                };
                crate::builders::pack_any(&item, "kiapi.board.types.Pad")
            })
            .collect();
        let definition = kiapi::board::types::Footprint {
            id: Some(kiapi::common::types::LibraryIdentifier {
                library_nickname: library_nickname.to_string(),
                entry_name: entry_name.to_string(),
            }),
            reference_field: Some(reference_field.clone()),
            value_field: Some(value_field.clone()),
            items: child_items,
            ..Default::default()
        };
        let footprint = kiapi::board::types::FootprintInstance {
            position: Some(crate::builders::vec2(x, y)),
            orientation: Some(kiapi::common::types::Angle {
                value_degrees: rotation,
            }),
            layer: crate::builders::layer_from_name(layer) as i32,
            locked: kiapi::common::types::LockedState::LsUnlocked as i32,
            definition: Some(definition),
            reference_field: Some(reference_field),
            value_field: Some(value_field),
            ..Default::default()
        };
        self.create_items(vec![crate::builders::pack_any(
            &footprint,
            "kiapi.board.types.FootprintInstance",
        )])?;
        let footprints = self.list_footprints()?;
        footprints
            .iter()
            .find(|footprint| footprint.reference == reference)
            .cloned()
            .with_context(|| {
                let references = footprints
                    .iter()
                    .map(|footprint| footprint.reference.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "KiCAD created the footprint but reference '{reference}' was not found (board references: {references})"
                )
            })
    }

    /// Get board extents (bounding box of all items).
    pub fn get_board_extents(&self) -> Result<IpcBoardExtents> {
        // Use GetBoundingBox with no specific items = board extents
        let header = self.make_header()?;
        let cmd = kiapi::common::commands::GetBoundingBox {
            header: Some(header),
            items: vec![], // empty = all items
            mode: kiapi::common::commands::BoundingBoxMode::BbmItemOnly as i32,
        };
        let resp_any = self.send_command(&cmd, "kiapi.common.commands.GetBoundingBox")?;
        if let Some(any) = resp_any {
            let resp: kiapi::common::commands::GetBoundingBoxResponse = unpack_any(&any)?;
            if let Some(bbox) = resp.boxes.first() {
                let pos = bbox.position.as_ref();
                let size = bbox.size.as_ref();
                return Ok(IpcBoardExtents {
                    min: IpcVector2 {
                        x: pos
                            .map(|p| crate::builders::nm_to_mm(p.x_nm))
                            .unwrap_or(0.0),
                        y: pos
                            .map(|p| crate::builders::nm_to_mm(p.y_nm))
                            .unwrap_or(0.0),
                    },
                    max: IpcVector2 {
                        x: pos
                            .map(|p| crate::builders::nm_to_mm(p.x_nm))
                            .unwrap_or(0.0)
                            + size
                                .map(|s| crate::builders::nm_to_mm(s.x_nm))
                                .unwrap_or(0.0),
                        y: pos
                            .map(|p| crate::builders::nm_to_mm(p.y_nm))
                            .unwrap_or(0.0)
                            + size
                                .map(|s| crate::builders::nm_to_mm(s.y_nm))
                                .unwrap_or(0.0),
                    },
                });
            }
        }
        anyhow::bail!("No bounding box returned from KiCAD")
    }

    /// Get enabled layers.
    pub fn get_layers(&self) -> Result<Vec<IpcLayer>> {
        let doc = self.get_board_document()?;
        let cmd = kiapi::board::commands::GetBoardEnabledLayers { board: Some(doc) };
        let resp_any = self.send_command(&cmd, "kiapi.board.commands.GetBoardEnabledLayers")?;
        if let Some(any) = resp_any {
            let resp: kiapi::board::commands::BoardEnabledLayersResponse = unpack_any(&any)?;
            let layers = resp
                .layers
                .iter()
                .map(|&l| {
                    let bl = kiapi::board::types::BoardLayer::try_from(l)
                        .unwrap_or(kiapi::board::types::BoardLayer::BlUndefined);
                    IpcLayer {
                        name: bl
                            .as_str_name()
                            .trim_start_matches("BL_")
                            .replace('_', ".")
                            .to_string(),
                        id: l,
                        kind: String::new(),
                    }
                })
                .collect();
            Ok(layers)
        } else {
            Ok(vec![])
        }
    }

    /// Run an arbitrary tool action in KiCAD (e.g. to trigger a refresh).
    pub fn run_action(&self, action: &str) -> Result<()> {
        let cmd = kiapi::common::commands::RunAction {
            action: action.to_string(),
        };
        self.send_command(&cmd, "kiapi.common.commands.RunAction")?;
        Ok(())
    }
}
