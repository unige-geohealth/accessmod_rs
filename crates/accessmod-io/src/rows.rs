//! Paged attribute rows of a vector layer or a CSV table, for the tables of
//! the interface: a page is bounded, whatever the size of the dataset.

use std::path::Path;

use gdal::vector::LayerAccess;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};
use crate::facilities::single_point;
use crate::output::read_csv;
use crate::tiles::PointSampler;
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
    /// Values of the sampled rasters at the point of each row, in the order
    /// requested (`null`: no point, outside the raster or NoData). Empty
    /// when no raster is sampled.
    #[serde(default)]
    pub samples: Vec<Vec<Option<f64>>>,
    /// Rows before this page.
    pub offset: u64,
    /// Rows matching the filter, every page included.
    pub total: u64,
}

/// Rasters sampled per row at most.
pub const MAX_SAMPLED: usize = 4;

/// Page of the attributes of the first layer of vector `path`, geometries
/// left out, in layer order. `filter` is an OGR SQL `WHERE`. Each raster of
/// `sample` is read at the point of each row (a point or one-point
/// multipoint), e.g. land cover and population at facilities.
pub fn vector_rows(
    path: &Path,
    filter: Option<&str>,
    offset: u64,
    limit: usize,
    sample: &[&Path],
) -> Result<Rows> {
    let limit = limit.min(MAX_PAGE);
    let dataset = open_dataset(path)?;
    let invalid = |issue: String| Error::invalid(format!("{}: {issue}", path.display()));
    let mut layer = dataset
        .layer(0)
        .map_err(|_| invalid("has no layer".into()))?;
    if let Some(filter) = filter {
        set_filter(&dataset, &mut layer, filter).map_err(invalid)?;
    }
    if sample.len() > MAX_SAMPLED {
        return Err(invalid(format!(
            "more than {MAX_SAMPLED} rasters to sample"
        )));
    }
    let samplers = sample
        .iter()
        .map(|raster| PointSampler::open(raster, layer.spatial_ref()))
        .collect::<Result<Vec<_>>>()?;
    let columns: Vec<String> = layer.defn().fields().map(|f| f.name()).collect();
    let mut rows = Vec::new();
    let mut samples = Vec::new();
    let mut total = 0u64;
    for feature in layer.features() {
        if total >= offset && rows.len() < limit {
            let mut row = Vec::with_capacity(columns.len());
            for index in 0..columns.len() {
                let value = feature.field(index).map_err(Error::gdal(path))?;
                row.push(json_attribute(value).unwrap_or(Value::Null));
            }
            rows.push(row);
            if !samplers.is_empty() {
                let point = feature.geometry().and_then(|g| single_point(g).ok());
                let values = samplers
                    .iter()
                    .map(|sampler| match point {
                        Some((x, y)) => sampler.sample(x, y),
                        None => Ok(None),
                    })
                    .collect::<Result<Vec<_>>>()?;
                samples.push(values);
            }
        }
        total += 1;
    }
    Ok(Rows {
        columns,
        rows,
        samples,
        offset,
        total,
    })
}

/// Distinct values of a field at most ([`field_values`]).
pub const MAX_VALUES: usize = 1000;

/// One distinct value of a field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldValue {
    /// The value (`null`: features without one).
    pub value: Value,
    /// The label of its first feature, when a label field is given.
    pub label: Option<String>,
    /// Features holding it.
    pub count: u64,
}

/// Distinct values of a field of a vector layer, e.g. the classes of a
/// road layer and their labels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FieldValues {
    /// Values, numbers ascending, then text, then `null`.
    pub values: Vec<FieldValue>,
    /// False when the layer has more than [`MAX_VALUES`] distinct values:
    /// `values` then holds the first ones met.
    pub complete: bool,
}

/// Distinct values of `field` in the first layer of vector `path`, with
/// the first value of `label` for each; one pass, memory bounded by
/// [`MAX_VALUES`].
pub fn field_values(path: &Path, field: &str, label: Option<&str>) -> Result<FieldValues> {
    let dataset = open_dataset(path)?;
    let invalid = |issue: String| Error::invalid(format!("{}: {issue}", path.display()));
    let mut layer = dataset
        .layer(0)
        .map_err(|_| invalid("has no layer".into()))?;
    let index = |name: &str| {
        layer
            .defn()
            .fields()
            .position(|f| f.name() == name)
            .ok_or_else(|| invalid(format!("no field {name:?}")))
    };
    let field_index = index(field)?;
    let label_index = label.map(index).transpose()?;
    let mut seen: std::collections::HashMap<String, FieldValue> = std::collections::HashMap::new();
    let mut complete = true;
    for feature in layer.features() {
        let value = json_attribute(feature.field(field_index).map_err(Error::gdal(path))?)
            .unwrap_or(Value::Null);
        let key = value.to_string();
        if let Some(entry) = seen.get_mut(&key) {
            entry.count += 1;
            continue;
        }
        if seen.len() >= MAX_VALUES {
            complete = false;
            continue;
        }
        let label = match label_index {
            Some(i) => {
                json_attribute(feature.field(i).map_err(Error::gdal(path))?).map(|v| match v {
                    Value::String(s) => s,
                    other => other.to_string(),
                })
            }
            None => None,
        };
        seen.insert(
            key,
            FieldValue {
                value,
                label,
                count: 1,
            },
        );
    }
    let mut values: Vec<FieldValue> = seen.into_values().collect();
    values.sort_by(|a, b| order(&a.value, &b.value));
    Ok(FieldValues { values, complete })
}

/// Numbers ascending, then text, then `null`.
fn order(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let rank = |v: &Value| match v {
        Value::Number(_) => 0,
        Value::Null => 2,
        _ => 1,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x
            .as_f64()
            .unwrap_or(f64::NAN)
            .total_cmp(&y.as_f64().unwrap_or(f64::NAN)),
        _ => rank(a).cmp(&rank(b)).then_with(|| match (a, b) {
            (Value::String(x), Value::String(y)) => x.cmp(y),
            _ => Ordering::Equal,
        }),
    }
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
        samples: Vec::new(),
        offset,
        total,
    })
}

/// Bytes of a table converted to JSON at most.
pub const MAX_JSON_BYTES: usize = 64 << 20;

/// CSV table `path` as a JSON array of objects keyed by column, values as
/// text; larger than [`MAX_JSON_BYTES`] is an error (the CSV itself has
/// no limit).
pub fn table_json(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path).map_err(Error::file(path))?;
    let too_large = || {
        Error::invalid(format!(
            "{}: larger than {} MiB as JSON; download it as CSV",
            path.display(),
            MAX_JSON_BYTES >> 20
        ))
    };
    let mut columns: Option<Vec<String>> = None;
    let mut out = b"[".to_vec();
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
        let object: serde_json::Map<String, Value> = columns
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    c.clone(),
                    record
                        .get(i)
                        .map_or(Value::Null, |v| Value::from(v.as_str())),
                )
            })
            .collect();
        if out.len() > 1 {
            out.push(b',');
        }
        serde_json::to_writer(&mut out, &object).map_err(|e| Error::invalid(e.to_string()))?;
        if out.len() > MAX_JSON_BYTES {
            return Err(too_large());
        }
        Ok(())
    })
    .map_err(|e| Error::invalid(format!("{}: {e}", path.display())))?;
    if columns.is_none() {
        return Err(Error::invalid(format!("{}: empty table", path.display())));
    }
    out.push(b']');
    Ok(out)
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
        let page = vector_rows(&path, None, 20, 10, &[]).expect("page");
        assert_eq!(page.columns, ["id"]);
        assert_eq!(page.total, 25);
        let ids: Vec<i64> = page
            .rows
            .iter()
            .map(|r| r[0].as_i64().expect("int"))
            .collect();
        assert_eq!(ids, [20, 21, 22, 23, 24]);

        let page = vector_rows(&path, Some("id < 3"), 0, 10, &[]).expect("filtered");
        assert_eq!((page.total, page.rows.len()), (3, 3));

        let page = vector_rows(&path, None, 0, usize::MAX, &[]).expect("capped");
        assert_eq!(page.rows.len(), 25);

        let err = vector_rows(&path, Some("id <<< 3"), 0, 10, &[]).expect_err("bad filter");
        assert!(err.to_string().contains("invalid filter"), "{err}");
        let err = vector_rows(&path, Some("nope = 1"), 0, 10, &[]).expect_err("unknown field");
        assert!(err.to_string().contains("invalid filter"), "{err}");
    }

    #[test]
    fn rasters_are_sampled_at_points() {
        use crate::fixtures::{X0, Y0, geo_transform, grid, write_raster};
        let dir = tempfile::tempdir().expect("temp dir");
        let raster = dir.path().join("lc.tif");
        write_raster::<u8>(
            &raster,
            3,
            2,
            geo_transform(X0, Y0),
            vec![1, 2, 3, 0, 5, 6],
            Some(0.0),
        );
        let (x, y) = grid(3, 2).cell_centre(0, 2);
        let (xn, yn) = grid(3, 2).cell_centre(1, 0);
        let path = dir.path().join("hf.gpkg");
        let inside = format!("POINT ({x} {y})");
        let nodata = format!("POINT ({xn} {yn})");
        let outside = format!("POINT ({} {y})", X0 - 50.0);
        let features: Vec<(Option<FieldValue>, &str)> = [&inside, &nodata, &outside]
            .iter()
            .enumerate()
            .map(|(i, wkt)| (Some(FieldValue::Integer64Value(i as i64)), wkt.as_str()))
            .collect();
        write_layer(
            &path,
            "hf",
            EPSG,
            ("id", OGRFieldType::OFTInteger64),
            &features,
        );
        let page = vector_rows(&path, None, 0, 10, &[&raster, &raster]).expect("page");
        assert_eq!(
            page.samples,
            [
                vec![Some(3.0), Some(3.0)],
                vec![None, None],
                vec![None, None]
            ]
        );
        let many = [raster.as_path(); MAX_SAMPLED + 1];
        assert!(vector_rows(&path, None, 0, 10, &many).is_err());
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
        let json: Value = serde_json::from_slice(&table_json(&path).expect("json")).expect("parse");
        assert_eq!(
            json,
            serde_json::json!([
                {"class": "1", "label": "a, b"},
                {"class": "002", "label": null},
                {"class": "3", "label": "c"}
            ])
        );
        std::fs::write(&path, "").expect("write");
        assert!(table_rows(&path, 0, 10).is_err());
        assert!(table_json(&path).is_err());
    }
}
