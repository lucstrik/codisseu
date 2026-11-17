//! SQL popup presentation only. Catalog lookup never requests more metadata.
//! Layout reference: saved PS-262.10968.76 completion captures at scale 1.
use crate::{
    Tokens, color,
    components::{self, Icon},
};
use codisseu_core::{
    remote::{ConnectionConfig, Engine, RemoteColumn},
    sql_completion::{Context, Kind, Suggestion, relation_namespace},
};
use egui::{FontId, Rect, TextFormat, Ui, text::LayoutJob};
use std::collections::BTreeMap;

pub(super) struct Row {
    suffix: String,
    detail: String,
    primary_key: bool,
}

impl Row {
    pub(super) fn new(
        item: &Suggestion,
        context: &Context,
        config: Option<&ConnectionConfig>,
        columns: &BTreeMap<(String, String), Vec<RemoteColumn>>,
    ) -> Self {
        let mut row = Self {
            suffix: String::new(),
            detail: item.detail.clone(),
            primary_key: false,
        };
        match item.kind {
            Kind::Table => {
                if !item.detail.is_empty() {
                    row.suffix = format!(" ({})", item.detail);
                }
                row.detail = config.map_or_else(String::new, |config| config.name.clone());
            }
            Kind::Namespace => {
                row.detail = config.map_or_else(String::new, |config| config.name.clone());
            }
            Kind::Column => {
                let (data_type, alias) = item.detail.rsplit_once(" · ").map_or_else(
                    || {
                        (
                            item.detail.as_str(),
                            context.qualifiers.last().map_or("", String::as_str),
                        )
                    },
                    |(data_type, alias)| (data_type, alias),
                );
                row.detail = data_type.into();
                if !alias.is_empty() && !item.label.starts_with(&format!("{alias}.")) {
                    row.suffix = format!(" ({alias})");
                }
                let default = config.map_or("", |config| match config.engine {
                    Engine::Postgres | Engine::AuroraPostgres => "public",
                    _ => &config.database,
                });
                // Explicit schema.table wins over aliases, just as in the
                // completion resolver. Never guess a PK without cached proof.
                if context.qualifiers.len() >= 2 {
                    let namespace = &context.qualifiers[context.qualifiers.len() - 2];
                    let table = context.qualifiers.last().unwrap();
                    row.primary_key = columns
                        .get(&(namespace.clone(), table.clone()))
                        .and_then(|columns| columns.iter().find(|column| column.name == item.label))
                        .is_some_and(|column| column.primary_key);
                } else {
                    let mut matches = context
                        .relations
                        .iter()
                        .filter(|relation| {
                            alias.is_empty() || relation.alias == alias || relation.name == alias
                        })
                        .filter_map(|relation| {
                            let name = item
                                .label
                                .strip_prefix(&format!("{}.", relation.alias))
                                .unwrap_or(&item.label);
                            columns
                                .get(&(
                                    relation_namespace(relation, default),
                                    relation.name.clone(),
                                ))
                                .and_then(|columns| {
                                    columns.iter().find(|column| column.name == name)
                                })
                        });
                    if let Some(first) = matches.next() {
                        row.primary_key =
                            first.primary_key && matches.all(|column| column.primary_key);
                    }
                }
            }
            Kind::Keyword => {}
        }
        row
    }
    pub(super) fn preferred_width(
        &self,
        ui: &egui::Context,
        item: &Suggestion,
        font: &FontId,
    ) -> f32 {
        ui.fonts_mut(|fonts| {
            [
                format!("{}{}", item.label, self.suffix),
                self.detail.clone(),
            ]
            .iter()
            .map(|text| {
                fonts
                    .layout_no_wrap(text.clone(), font.clone(), egui::Color32::WHITE)
                    .size()
                    .x
            })
            .sum::<f32>()
                + 28.
                + 24.
        })
    }
    pub(super) fn paint(
        &self,
        ui: &mut Ui,
        rect: Rect,
        item: &Suggestion,
        prefix: &str,
        tokens: &Tokens,
    ) {
        let foreground = color(tokens.database_completion_foreground);
        let secondary = color(tokens.completion_footer_text);
        let icon_rect = Rect::from_center_size(
            rect.left_center() + egui::vec2(13., 0.),
            egui::vec2(16., 16.),
        );
        if item.kind == Kind::Column {
            components::paint_database_column(
                ui,
                icon_rect,
                self.primary_key,
                foreground,
                color(tokens.database_completion_key),
            );
        } else {
            let icon = match item.kind {
                Kind::Namespace => Icon::Schema,
                Kind::Table => Icon::Table,
                Kind::Keyword => Icon::Code,
                Kind::Column => unreachable!(),
            };
            components::paint_database_icon(ui, icon_rect, icon, foreground);
        }
        let font = FontId::monospace(tokens.editor_font_size);
        let detail = ui
            .fonts_mut(|fonts| fonts.layout_no_wrap(self.detail.clone(), font.clone(), secondary));
        let detail_width = detail.size().x.min(rect.width() * 0.44);
        let detail_rect = Rect::from_min_max(
            egui::pos2(rect.right() - detail_width - 8., rect.top()),
            rect.max,
        );
        let label_rect = Rect::from_min_max(
            rect.min,
            egui::pos2(detail_rect.left() - 12., rect.bottom()),
        );
        let mut job = LayoutJob::default();
        let match_start = if item
            .label
            .to_lowercase()
            .starts_with(&prefix.to_lowercase())
        {
            0
        } else {
            item.label.rfind('.').map_or(0, |dot| dot + 1)
        };
        let tail = &item.label[match_start..];
        let match_end = if tail.to_lowercase().starts_with(&prefix.to_lowercase()) {
            match_start
                + tail
                    .char_indices()
                    .nth(prefix.chars().count())
                    .map_or(tail.len(), |(byte, _)| byte)
        } else {
            match_start
        };
        let format = |color| TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        };
        job.append(&item.label[..match_start], 0., format(foreground));
        job.append(
            &item.label[match_start..match_end],
            0.,
            format(color(tokens.completion_match_color)),
        );
        job.append(&item.label[match_end..], 0., format(foreground));
        job.append(&self.suffix, 0., format(secondary));
        let label = ui.fonts_mut(|fonts| fonts.layout_job(job));
        ui.painter().with_clip_rect(label_rect).galley(
            rect.left_center() + egui::vec2(28., -label.size().y / 2.),
            label,
            foreground,
        );
        ui.painter().with_clip_rect(detail_rect).galley(
            egui::pos2(
                rect.right() - 8. - detail.size().x,
                rect.center().y - detail.size().y / 2.,
            ),
            detail,
            secondary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use codisseu_core::sql_text::SqlDialect;

    #[test]
    fn primary_key_decoration_requires_the_matching_cached_column() {
        let text = "SELECT r.i FROM corrier.remetentes r";
        let context = codisseu_core::sql_completion::context(text, 10, SqlDialect::Mysql).unwrap();
        let config = ConnectionConfig {
            database: String::new(),
            ..Default::default()
        };
        let item = Suggestion {
            label: "id".into(),
            insert: "`id`".into(),
            detail: "bigint".into(),
            kind: Kind::Column,
        };
        let columns = BTreeMap::from([(
            ("corrier".into(), "remetentes".into()),
            vec![RemoteColumn {
                name: "id".into(),
                data_type: "bigint".into(),
                primary_key: true,
                nullable: false,
            }],
        )]);
        let row = Row::new(&item, &context, Some(&config), &columns);
        assert!(row.primary_key);
        assert_eq!(row.suffix, " (r)");
        assert_eq!(row.detail, "bigint");
        assert!(!Row::new(&item, &context, Some(&config), &BTreeMap::new()).primary_key);
        let other = Suggestion {
            label: "name".into(),
            ..item.clone()
        };
        assert!(!Row::new(&other, &context, Some(&config), &columns).primary_key);
        assert_eq!(
            item.insert, "`id`",
            "presentation must not rewrite resolver insertions"
        );
    }

    #[test]
    fn table_decoration_separates_namespace_from_datasource_name() {
        let context = codisseu_core::sql_completion::context(
            "SELECT * FROM corrier.rem",
            25,
            SqlDialect::Mysql,
        )
        .unwrap();
        let config = ConnectionConfig {
            name: "Fixture MySQL".into(),
            ..Default::default()
        };
        let item = Suggestion {
            label: "remetentes".into(),
            insert: "`remetentes`".into(),
            detail: "corrier".into(),
            kind: Kind::Table,
        };
        let row = Row::new(&item, &context, Some(&config), &BTreeMap::new());
        assert_eq!(row.suffix, " (corrier)");
        assert_eq!(row.detail, "Fixture MySQL");
        assert!(!row.primary_key);
        assert_eq!(
            item.detail, "corrier",
            "presentation must preserve metadata used by the resolver"
        );
    }
}
