// Derived from Chunker (https://github.com/HiveGamesOSS/Chunker), Copyright (c) 2024 Hive Games, MIT License.
//! Signs and hanging signs (`JavaSignBlockEntityHandler`): the 1.20 `front_text`/`back_text` layout, or the front face
//! on the root with `Text1..4` before.

use serde_json::Value as Json;

use super::ctx::Ctx;
use crate::convert::model::blockentity::{Sign, SignFace};
use crate::convert::nbt::Compound;
use crate::convert::util::json_text;

pub(super) fn write(ctx: &Ctx, out: &mut Compound, sign: &Sign) -> Option<()> {
    if ctx.since(1, 20, 0) {
        out.put("is_waxed", sign.waxed);
        if ctx.since(26, 3, 0) {
            out.put("allow_op_features", sign.allow_op_features);
        }
        let mut front = Compound::new();
        write_face(ctx, &mut front, &sign.front)?;
        out.put("front_text", front);
        let mut back = Compound::new();
        write_face(ctx, &mut back, &sign.back)?;
        out.put("back_text", back);
    } else {
        write_face(ctx, out, &sign.front)?;
    }
    Some(())
}

fn color_name(color: i32) -> &'static str {
    json_text::color_of_bedrock_hex(color as u32).unwrap_or("black")
}

fn write_face(ctx: &Ctx, out: &mut Compound, face: &SignFace) -> Option<()> {
    if ctx.since(1, 20, 0) {
        out.put("messages", ctx.text_list(&face.lines, 4)?);
        out.put("has_glowing_text", face.lit);
        out.put("color", color_name(face.color));
    } else {
        for i in 0..4 {
            let empty: Json = json_text::empty_text();
            out.put(format!("Text{}", i + 1), ctx.text(face.lines.get(i).unwrap_or(&empty))?);
        }
        if ctx.since(1, 17, 0) {
            out.put("GlowingText", face.lit);
        }
        out.put("Color", color_name(face.color));
    }
    Some(())
}
