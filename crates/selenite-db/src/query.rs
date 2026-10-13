use std::{cmp::Ordering, collections::BTreeSet};

use anyhow::{Result, ensure};

use crate::model::{
    Cell, DatabaseSchema, Filter, Layout, PropertyType, QueryResult, Row, RowGroup, SavedView,
    validate_id,
};

impl SavedView {
    pub fn validate(&self, schema: &DatabaseSchema) -> Result<()> {
        schema.validate()?;
        validate_id(&self.id)?;
        ensure!(!self.name.trim().is_empty(), "view name cannot be empty");
        ensure!(
            self.database == schema.id,
            "view belongs to a different database"
        );
        if let Some(filter) = &self.filter {
            validate_filter(schema, filter, 0)?;
        }
        let mut sorted = BTreeSet::new();
        for sort in &self.sort {
            schema.field_type(&sort.field)?;
            ensure!(
                sorted.insert(&sort.field),
                "duplicate sort field {}",
                sort.field
            );
        }
        if let Some(field) = &self.group_by {
            schema.field_type(field)?;
        }
        if let Some(field) = &self.calendar_field {
            ensure!(
                schema.field_type(field)? == PropertyType::Date,
                "calendar field must have date type"
            );
        }
        if let Some(field) = &self.gallery_field {
            ensure!(
                matches!(
                    schema.field_type(field)?,
                    PropertyType::File | PropertyType::Url
                ),
                "gallery field must have file or URL type"
            );
        }
        match self.layout {
            Layout::Board => ensure!(self.group_by.is_some(), "board layout needs group_by"),
            Layout::Calendar => ensure!(
                self.calendar_field.is_some(),
                "calendar layout needs calendar_field"
            ),
            Layout::Table | Layout::Gallery => {}
        }
        Ok(())
    }
}

fn operand(schema: &DatabaseSchema, field: &str, value: &Cell) -> Result<()> {
    if matches!(field, "$title" | "$path") {
        ensure!(
            matches!(value, Cell::Text(_) | Cell::Empty),
            "page metadata operands must be text"
        );
        Ok(())
    } else {
        let mut field = schema.field(field)?.clone();
        // Filter operands do not create a row and may ask about missing values.
        field.required = false;
        field.validate_cell(value)
    }
}

fn validate_filter(schema: &DatabaseSchema, filter: &Filter, depth: usize) -> Result<()> {
    ensure!(depth <= 32, "filters may be nested at most 32 levels");
    match filter {
        Filter::All { filters } | Filter::Any { filters } => {
            for filter in filters {
                validate_filter(schema, filter, depth + 1)?;
            }
        }
        Filter::Eq { field, value } => operand(schema, field, value)?,
        Filter::Contains { field, .. } => ensure!(
            matches!(
                schema.field_type(field)?,
                PropertyType::Text
                    | PropertyType::Select
                    | PropertyType::MultiSelect
                    | PropertyType::Relation
                    | PropertyType::Url
                    | PropertyType::File
            ),
            "contains needs a text or list field"
        ),
        Filter::Range { field, min, max } => {
            ensure!(
                matches!(
                    schema.field_type(field)?,
                    PropertyType::Number | PropertyType::Date
                ),
                "range needs a number or date field"
            );
            ensure!(
                min.is_some() || max.is_some(),
                "range needs at least one bound"
            );
            for value in min.iter().chain(max) {
                ensure!(
                    !matches!(value, Cell::Empty),
                    "range bounds cannot be empty"
                );
                operand(schema, field, value)?;
            }
            if let (Some(min), Some(max)) = (min, max) {
                ensure!(
                    compare(min, max) != Ordering::Greater,
                    "range minimum exceeds maximum"
                );
            }
        }
        Filter::IsEmpty { field } => {
            schema.field_type(field)?;
        }
    }
    Ok(())
}

fn matches_filter(row: &Row, filter: &Filter) -> bool {
    match filter {
        Filter::All { filters } => filters.iter().all(|filter| matches_filter(row, filter)),
        Filter::Any { filters } => filters.iter().any(|filter| matches_filter(row, filter)),
        Filter::Eq { field, value } => row.cell(field) == *value,
        Filter::Contains { field, value } => match row.cell(field) {
            Cell::MultiSelect(values) | Cell::Relation(values) => values.contains(value),
            cell => cell.text().is_some_and(|text| text.contains(value)),
        },
        Filter::Range { field, min, max } => {
            let value = row.cell(field);
            !value.is_empty()
                && min
                    .as_ref()
                    .is_none_or(|min| compare(&value, min) != Ordering::Less)
                && max
                    .as_ref()
                    .is_none_or(|max| compare(&value, max) != Ordering::Greater)
        }
        Filter::IsEmpty { field } => row.cell(field).is_empty(),
    }
}

/// Validate a saved view and evaluate it against typed rows. Sort ties always
/// use the page path, independent of filesystem enumeration or previous views.
pub fn query_rows(
    schema: &DatabaseSchema,
    view: &SavedView,
    mut rows: Vec<Row>,
) -> Result<QueryResult> {
    view.validate(schema)?;
    let mut paths = BTreeSet::new();
    for row in &rows {
        ensure!(paths.insert(&row.path), "duplicate row path {}", row.path);
        for field in &schema.fields {
            field.validate_cell(&row.cell(&field.name))?;
        }
    }
    if let Some(filter) = &view.filter {
        rows.retain(|row| matches_filter(row, filter));
    }
    rows.sort_by(|left, right| {
        for sort in &view.sort {
            let left = left.cell(&sort.field);
            let right = right.cell(&sort.field);
            // Missing/empty values stay last in either direction.
            let order = match (left.is_empty(), right.is_empty()) {
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (true, true) => Ordering::Equal,
                (false, false) => {
                    let order = compare(&left, &right);
                    if sort.descending {
                        order.reverse()
                    } else {
                        order
                    }
                }
            };
            if order != Ordering::Equal {
                return order;
            }
        }
        left.path.cmp(&right.path)
    });
    let mut groups: Vec<RowGroup> = Vec::new();
    if let Some(field) = &view.group_by {
        for row in &rows {
            let value = row.cell(field);
            let keys = if value.is_empty() {
                vec![Cell::Empty]
            } else {
                match value {
                    Cell::MultiSelect(values) => values.into_iter().map(Cell::Select).collect(),
                    Cell::Relation(values) => values.into_iter().map(Cell::Text).collect(),
                    value => vec![value],
                }
            };
            for key in keys {
                if let Some(group) = groups.iter_mut().find(|group| group.key == key) {
                    group.rows.push(row.path.clone());
                } else {
                    groups.push(RowGroup {
                        key,
                        rows: vec![row.path.clone()],
                    });
                }
            }
        }
        groups.sort_by(|left, right| compare(&left.key, &right.key));
    }
    Ok(QueryResult {
        schema: schema.clone(),
        view: view.clone(),
        rows,
        groups,
    })
}

fn compare(left: &Cell, right: &Cell) -> Ordering {
    match (left, right) {
        (Cell::Empty, Cell::Empty) => Ordering::Equal,
        (Cell::Empty, _) => Ordering::Greater,
        (_, Cell::Empty) => Ordering::Less,
        (Cell::Number(left), Cell::Number(right)) => left.total_cmp(right),
        (Cell::Checkbox(left), Cell::Checkbox(right)) => left.cmp(right),
        (Cell::MultiSelect(left), Cell::MultiSelect(right))
        | (Cell::Relation(left), Cell::Relation(right)) => left.cmp(right),
        _ => left.text().cmp(&right.text()),
    }
}
