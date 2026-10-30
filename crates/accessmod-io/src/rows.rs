//! Paged attribute rows of a vector layer or a CSV table, for the tables of
//! the interface: a page is bounded, whatever the size of the dataset.

use std::path::Path;

use gdal::vector::LayerAccess;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::output::read_csv;
use crate::vector::{json_attribute, open_dataset, set_filter};

/// Rows of a page at most.
pub const MAX_PAGE: usize = 1000;

/// Bytes of a CSV record at most.
pub const MAX_RECORD_BYTES: usize = 1 << 20;

/// One page of rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rows {
    /// Column names, in order.
    pub columns: Vec<String>,
    /// Values of each row, in column order (`null`: no value). Table values
    /// are text, as stored.
    pub rows: Vec<Vec<Value>>,
    /// Rows before this page.
    pub offset: u64,
    /// Rows matching the filter, every page included.
    pub total: u64,
}

/// Page of the attributes of the first layer of vector `path`, geometries
/// left out, in layer order. `filter` is an OGR SQL `WHERE`.
pub fn vector_rows(path: &Path, filter: Option<&str>, offset: u64, limit: usize) -> Result<Rows> {
    let limit = limit.min(MAX_PAGE);
    let dataset = open_dataset(path)?;
    let invalid = |issue: String| Error::invalid(format!("{}: {issue}", path.display()));
    let mut layer = dataset
        .layer(0)
        .map_err(|_| invalid("has no layer".into()))?;
    if let Some(filter) = filter {
        set_filter(&dataset, &mut layer, filter).map_err(invalid)?;
    }
    let columns: Vec<String> = layer.defn().fields().map(|f| f.name()).collect();
    let mut rows = Vec::new();
    let mut total = 0u64;
    for feature in layer.features() {
        if total >= offset && rows.len() < limit {
            let mut row = Vec::with_capacity(columns.len());
            for index in 0..columns.len() {
                let value = feature.field(index).map_err(Error::gdal(path))?;
                row.push(json_attribute(value).unwrap_or(Value::Null));
            }
            rows.push(row);
        }
        total += 1;
    }
    Ok(Rows {
        columns,
        rows,
        offset,
        total,
    })
}

/// Page of the records of CSV table `path` (header first, as the project
/// stores tables).
pub fn table_rows(path: &Path, offset: u64, limit: usize) -> Result<Rows> {
    let limit = limit.min(MAX_PAGE);
    let file = std::fs::File::open(path).map_err(Error::file(path))?;
    let mut columns: Option<Vec<String>> = None;
    let mut rows = Vec::new();
    let mut total = 0u64;
    read_csv(file, MAX_RECORD_BYTES, |record| {
        let Some(columns) = &columns else {
            columns = Some(
                record
                    .iter()
                    .map(|c| c.trim_start_matches('\u{feff}').trim().to_owned())
                    .collect(),
            );
            return Ok(());
        };
        if total >= offset && rows.len() < limit {
            let mut row: Vec<Value> = record.iter().map(|v| Value::from(v.as_str())).collect();
            row.resize(columns.len(), Value::Null);
            rows.push(row);
        }
        total += 1;
        Ok(())
    })
    .map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    let columns =
        columns.ok_or_else(|| Error::invalid(format!("{}: empty table", path.display())))?;
    Ok(Rows {
        columns,
        rows,
        offset,
        total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::{EPSG, write_layer};
    use gdal::vector::{FieldValue, OGRFieldType};

    fn layer(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("hf.gpkg");
        let features: Vec<(Option<FieldValue>, &str)> = (0..25)
            .map(|i| (Some(FieldValue::Integer64Value(i)), "POINT (500000 0)"))
            .collect();
        write_layer(
            &path,
            "hf",
            EPSG,
            ("id", OGRFieldType::OFTInteger64),
            &features,
        );
        path
    }

    #[test]
    fn vector_pages_are_bounded_and_filtered() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = layer(dir.path());
        let page = vector_rows(&path, None, 20, 10).expect("page");
        assert_eq!(page.columns, ["id"]);
        assert_eq!(page.total, 25);
        let ids: Vec<i64> = page
            .rows
            .iter()
            .map(|r| r[0].as_i64().expect("int"))
            .collect();
        assert_eq!(ids, [20, 21, 22, 23, 24]);

        let page = vector_rows(&path, Some("id < 3"), 0, 10).expect("filtered");
        assert_eq!((page.total, page.rows.len()), (3, 3));

        let page = vector_rows(&path, None, 0, usize::MAX).expect("capped");
        assert_eq!(page.rows.len(), 25);

        let err = vector_rows(&path, Some("id <<< 3"), 0, 10).expect_err("bad filter");
        assert!(err.to_string().contains("invalid filter"), "{err}");
        let err = vector_rows(&path, Some("nope = 1"), 0, 10).expect_err("unknown field");
        assert!(err.to_string().contains("invalid filter"), "{err}");
    }

    #[test]
    fn table_pages_keep_text_and_pad_short_records() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("t.csv");
        std::fs::write(&path, "\u{feff}class,label\n1,\"a, b\"\n002\n3,c\n").expect("write");
        let page = table_rows(&path, 1, 10).expect("page");
        assert_eq!(page.columns, ["class", "label"]);
        assert_eq!(page.total, 3);
        assert_eq!(
            page.rows,
            [
                vec![Value::from("002"), Value::Null],
                vec![Value::from("3"), Value::from("c")]
            ]
        );
        std::fs::write(&path, "").expect("write");
        assert!(table_rows(&path, 0, 10).is_err());
    }
}
