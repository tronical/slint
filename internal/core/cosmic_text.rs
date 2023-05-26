// Copyright © SixtyFPS GmbH <info@slint-ui.com>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-commercial

use i_slint_common::sharedfontdb;

use crate::graphics::FontRequest;
use crate::lengths::{LogicalLength, PhysicalPx, ScaleFactor};

type PhysicalLength = euclid::Length<f32, PhysicalPx>;

pub struct TextLayout {
    pub buffer: cosmic_text::Buffer,
}

impl TextLayout {
    pub fn new(
        text: &str,
        font_request: &FontRequest,
        scale_factor: ScaleFactor,
        default_font_size: LogicalLength,
        max_width: Option<PhysicalLength>,
        max_height: PhysicalLength,
    ) -> Self {
        sharedfontdb::FONT_DB.with(|db| {
            let mut db = db.borrow_mut();
            let mut font_system = &mut db.font_system;

            // TODO:
            // text alignment (horizontal and vertical)
            // overflow handling
            // wrap / no-wrap

            let pixel_size = font_request.pixel_size.unwrap_or(default_font_size) * scale_factor;

            // apply correct font to attributes, etc.
            let mut buffer = cosmic_text::Buffer::new(
                &mut font_system,
                cosmic_text::Metrics { font_size: pixel_size.get(), line_height: pixel_size.get() },
            );
            buffer.set_text(
                &mut font_system,
                text,
                cosmic_text::Attrs::new(),
                cosmic_text::Shaping::Advanced,
            );
            buffer.set_size(
                &mut font_system,
                max_width.map_or(f32::MAX, |w| w.get()),
                max_height.get(),
            );

            Self { buffer }
        })
    }
}
