//! Projections for new projects: candidates for a region, and how much they
//! distort distances and areas over it (`docs/project-creator.md`,
//! contract 3).
//!
//! The travel-time solver uses one cell size everywhere, so the **distance
//! scale** of the projection over the region bounds the error of every time
//! and distance; the **area scale** bounds the error of areas and densities
//! (people counts are kept by summing, whatever the projection). Both come
//! from Tissot's indicatrix, computed by finite differences through the
//! projection itself on a lattice of points over the region, on the WGS 84
//! ellipsoid: any projected CRS PROJ knows can be measured, national grids
//! included.

use gdal::spatial_ref::{AxisMappingStrategy, CoordTransform, SpatialRef};

use crate::error::{Error, Result};
use crate::grid::check_projected_metric;

/// WGS 84 semi-major axis, metres.
const A: f64 = 6_378_137.0;
/// WGS 84 first eccentricity squared.
const E2: f64 = 0.006_694_379_990_141_317;
/// Step of the finite differences, degrees (about 1 m).
const STEP: f64 = 1e-5;
/// Points per side of the lattice over a region.
const SAMPLES: usize = 21;

/// A region in WGS 84 degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LonLatBounds {
    /// West longitude.
    pub west: f64,
    /// South latitude.
    pub south: f64,
    /// East longitude.
    pub east: f64,
    /// North latitude.
    pub north: f64,
}

impl LonLatBounds {
    /// Why the bounds cannot hold a project, if they cannot (v1: no
    /// antimeridian crossing, nothing beyond 80°).
    pub fn check(&self) -> Option<String> {
        let finite = [self.west, self.south, self.east, self.north]
            .iter()
            .all(|v| v.is_finite());
        if !finite || self.west >= self.east || self.south >= self.north {
            return Some("the region is empty".to_owned());
        }
        if self.west < -180.0 || self.east > 180.0 {
            return Some("the region crosses the antimeridian: not supported".to_owned());
        }
        if self.south < -80.0 || self.north > 80.0 {
            return Some("the region reaches beyond 80° of latitude: not supported".to_owned());
        }
        None
    }

    fn centre(&self) -> (f64, f64) {
        (
            (self.west + self.east) / 2.0,
            (self.south + self.north) / 2.0,
        )
    }
}

/// Distortion of a projection over a region: the largest relative errors,
/// e.g. `0.004` for 0.4 %.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct Distortion {
    /// Largest error of a distance, in any direction.
    pub distance: f64,
    /// Largest error of an area.
    pub area: f64,
}

/// Distortion of `srs` over `bounds`, measured on a lattice of points
/// (edges included).
pub fn distortion(srs: &SpatialRef, bounds: &LonLatBounds) -> Result<Distortion> {
    if let Some(problem) = bounds.check() {
        return Err(Error::invalid(problem));
    }
    let issues = check_projected_metric(Some(srs));
    if !issues.is_empty() {
        return Err(Error::Validation { issues });
    }
    let mut wgs84 = SpatialRef::from_epsg(4326).map_err(Error::gdal("EPSG:4326"))?;
    wgs84.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    let mut target = srs.clone();
    target.set_axis_mapping_strategy(AxisMappingStrategy::TraditionalGisOrder);
    let transform = CoordTransform::new(&wgs84, &target).map_err(Error::gdal("projection"))?;
    // Each sample and its two neighbours, east and north.
    let mut xs = Vec::with_capacity(SAMPLES * SAMPLES * 3);
    let mut ys = Vec::with_capacity(SAMPLES * SAMPLES * 3);
    let at = |i: usize, lo: f64, hi: f64| lo + (hi - lo) * i as f64 / (SAMPLES - 1) as f64;
    // Neighbours stay inside the bounds: step inwards on the east and north
    // edges.
    for i in 0..SAMPLES {
        for j in 0..SAMPLES {
            let lon = at(i, bounds.west, bounds.east - STEP);
            let lat = at(j, bounds.south, bounds.north - STEP);
            xs.extend([lon, lon + STEP, lon]);
            ys.extend([lat, lat, lat + STEP]);
        }
    }
    let (lons, lats) = (xs.clone(), ys.clone());
    transform
        .transform_coords(&mut xs, &mut ys, &mut [])
        .map_err(|_| Error::invalid("the projection cannot represent the whole region"))?;
    let mut worst = Distortion {
        distance: 0.0,
        area: 0.0,
    };
    for k in (0..xs.len()).step_by(3) {
        let phi = lats[k].to_radians();
        let step = STEP.to_radians();
        let (sin, cos) = phi.sin_cos();
        let w = (1.0 - E2 * sin * sin).sqrt();
        let m = A * (1.0 - E2) / (w * w * w); // meridian radius
        let n = A / w; // prime vertical radius
        // Partial derivatives per radian of longitude and latitude.
        let (x_lam, y_lam) = ((xs[k + 1] - xs[k]) / step, (ys[k + 1] - ys[k]) / step);
        let (x_phi, y_phi) = ((xs[k + 2] - xs[k]) / step, (ys[k + 2] - ys[k]) / step);
        let h = x_phi.hypot(y_phi) / m;
        let kk = x_lam.hypot(y_lam) / (n * cos);
        let s = (x_lam * y_phi - x_phi * y_lam).abs() / (m * n * cos);
        // Tissot's semi-axes a ≥ b: a² + b² = h² + k², a·b = s.
        let sum = h * h + kk * kk;
        let root = (sum * sum / 4.0 - s * s).max(0.0).sqrt();
        let a = (sum / 2.0 + root).sqrt();
        let b = (sum / 2.0 - root).max(0.0).sqrt();
        let values = [a, b, s, lons[k]];
        if !values.iter().all(|v| v.is_finite()) {
            return Err(Error::invalid(
                "the projection cannot represent the whole region",
            ));
        }
        worst.distance = worst.distance.max((a - 1.0).abs()).max((b - 1.0).abs());
        worst.area = worst.area.max((s - 1.0).abs());
    }
    Ok(worst)
}

/// A projection offered for a region.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// What it is, e.g. `UTM zone 36N`.
    pub name: String,
    /// The CRS.
    pub srs: SpatialRef,
    /// Its distortion over the region.
    pub distortion: Distortion,
}

/// UTM zone (1–60) of longitude `lon`.
pub fn utm_zone(lon: f64) -> u32 {
    (((lon + 180.0) / 6.0).floor() as i64).clamp(0, 59) as u32 + 1
}

fn proj(text: &str) -> Result<SpatialRef> {
    SpatialRef::from_proj4(text).map_err(Error::gdal("projection"))
}

/// The UTM CRS (WGS 84) of the zone holding the centre of `bounds`.
pub fn utm(bounds: &LonLatBounds) -> Result<(String, SpatialRef)> {
    let (lon, lat) = bounds.centre();
    let zone = utm_zone(lon);
    let (code, hemisphere) = if lat >= 0.0 {
        (32600 + zone, "N")
    } else {
        (32700 + zone, "S")
    };
    let srs = SpatialRef::from_epsg(code).map_err(Error::gdal("UTM"))?;
    Ok((format!("UTM zone {zone}{hemisphere} (EPSG:{code})"), srs))
}

/// A Transverse Mercator centred on `bounds` (conformal).
pub fn transverse_mercator(bounds: &LonLatBounds) -> Result<(String, SpatialRef)> {
    let (lon, lat) = bounds.centre();
    let srs = proj(&format!(
        "+proj=tmerc +lat_0={lat:.6} +lon_0={lon:.6} +k=1 +x_0=0 +y_0=0 +datum=WGS84 +units=m +no_defs"
    ))?;
    Ok((
        format!("Transverse Mercator centred on {lon:.2}°, {lat:.2}°"),
        srs,
    ))
}

/// A Lambert Azimuthal Equal Area centred on `bounds` (equal area).
pub fn lambert_azimuthal(bounds: &LonLatBounds) -> Result<(String, SpatialRef)> {
    let (lon, lat) = bounds.centre();
    let srs = proj(&format!(
        "+proj=laea +lat_0={lat:.6} +lon_0={lon:.6} +x_0=0 +y_0=0 +datum=WGS84 +units=m +no_defs"
    ))?;
    Ok((
        format!("Lambert Azimuthal Equal Area centred on {lon:.2}°, {lat:.2}°"),
        srs,
    ))
}

/// An Albers Equal Area for `bounds`, standard parallels at a sixth from
/// each edge (equal area).
pub fn albers(bounds: &LonLatBounds) -> Result<(String, SpatialRef)> {
    let (lon, lat) = bounds.centre();
    let span = bounds.north - bounds.south;
    let (lat1, lat2) = (bounds.south + span / 6.0, bounds.north - span / 6.0);
    let srs = proj(&format!(
        "+proj=aea +lat_0={lat:.6} +lon_0={lon:.6} +lat_1={lat1:.6} +lat_2={lat2:.6} \
         +x_0=0 +y_0=0 +datum=WGS84 +units=m +no_defs"
    ))?;
    Ok((format!("Albers Equal Area for {lat1:.2}°–{lat2:.2}°"), srs))
}

/// The projections offered for `bounds`, least distance distortion first:
/// UTM, a centred Transverse Mercator, Albers and Lambert Azimuthal (the
/// last two when exact areas matter more).
pub fn candidates(bounds: &LonLatBounds) -> Result<Vec<Candidate>> {
    if let Some(problem) = bounds.check() {
        return Err(Error::invalid(problem));
    }
    let mut out = Vec::new();
    for make in [utm, transverse_mercator, albers, lambert_azimuthal] {
        let (name, srs) = make(bounds)?;
        // A projection that cannot hold the region is left out.
        if let Ok(distortion) = distortion(&srs, bounds) {
            out.push(Candidate {
                name,
                srs,
                distortion,
            });
        }
    }
    out.sort_by(|a, b| a.distortion.distance.total_cmp(&b.distortion.distance));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A window of South Sudan (UTM 36N).
    const SSD: LonLatBounds = LonLatBounds {
        west: 33.0,
        south: 9.0,
        east: 33.5,
        north: 9.5,
    };

    fn epsg(code: u32) -> SpatialRef {
        SpatialRef::from_epsg(code).expect("srs")
    }

    #[test]
    fn utm_near_its_central_meridian_scales_by_0_9996() {
        // Zone 36 is centred on 33° E.
        let d = distortion(&epsg(32636), &SSD).expect("distortion");
        assert!((d.distance - 4e-4).abs() < 2e-5, "{d:?}");
        assert!(d.area < 8.1e-4, "{d:?}");
        assert_eq!(utm_zone(33.2), 36);
        assert_eq!(utm(&SSD).unwrap().0, "UTM zone 36N (EPSG:32636)");
    }

    #[test]
    fn conformal_and_equal_area_trade_differently() {
        // A wide region: 20° of longitude.
        let wide = LonLatBounds {
            west: 20.0,
            south: 0.0,
            east: 40.0,
            north: 15.0,
        };
        let (_, tm) = transverse_mercator(&wide).unwrap();
        let (_, laea) = lambert_azimuthal(&wide).unwrap();
        let (_, aea) = albers(&wide).unwrap();
        let tm = distortion(&tm, &wide).unwrap();
        let laea = distortion(&laea, &wide).unwrap();
        let aea = distortion(&aea, &wide).unwrap();
        // Equal area: areas exact, distances not.
        assert!(laea.area < 1e-6 && aea.area < 1e-6, "{laea:?} {aea:?}");
        assert!(laea.distance > 1e-3, "{laea:?}");
        // Conformal: scale error the same in every direction, areas twice it.
        assert!(tm.distance > 1e-3 && tm.area > tm.distance, "{tm:?}");
        // A small region in a centred TM is almost exact.
        let (_, small) = transverse_mercator(&SSD).unwrap();
        assert!(distortion(&small, &SSD).unwrap().distance < 1e-4);
    }

    #[test]
    fn national_grids_are_measured_too() {
        // Switzerland in LV95 (oblique Mercator).
        let ch = LonLatBounds {
            west: 6.0,
            south: 45.8,
            east: 10.5,
            north: 47.8,
        };
        let d = distortion(&epsg(2056), &ch).expect("LV95");
        assert!(d.distance < 3e-4, "{d:?}");
        // Web Mercator at 46° N: a projected metric CRS, badly distorted.
        let d = distortion(&epsg(3857), &ch).expect("3857");
        assert!(d.distance > 0.4, "{d:?}");
        // Geographic CRS are refused.
        assert!(distortion(&epsg(4326), &ch).is_err());
    }

    #[test]
    fn candidates_put_the_least_distortion_first() {
        let list = candidates(&SSD).expect("candidates");
        assert_eq!(list.len(), 4);
        assert!(list[0].distortion.distance <= list[1].distortion.distance);
        assert!(list.iter().any(|c| c.name.starts_with("UTM")));
    }

    #[test]
    fn unsupported_regions_are_refused() {
        let across = LonLatBounds {
            west: 170.0,
            south: -20.0,
            east: 190.0,
            north: -10.0,
        };
        assert!(across.check().unwrap().contains("antimeridian"));
        let polar = LonLatBounds { north: 85.0, ..SSD };
        assert!(polar.check().unwrap().contains("80°"));
        let empty = LonLatBounds { east: 33.0, ..SSD };
        assert!(candidates(&empty).is_err());
        let nan = LonLatBounds {
            west: f64::NAN,
            ..SSD
        };
        assert!(distortion(&epsg(32636), &nan).is_err());
    }
}
