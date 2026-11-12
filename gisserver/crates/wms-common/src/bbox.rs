//! Axis-aligned bounding box in CRS units.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle `[minx, miny, maxx, maxy]` in CRS units.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bbox {
    pub minx: f64,
    pub miny: f64,
    pub maxx: f64,
    pub maxy: f64,
}

impl Bbox {
    pub fn new(minx: f64, miny: f64, maxx: f64, maxy: f64) -> Self {
        Self {
            minx,
            miny,
            maxx,
            maxy,
        }
    }

    pub fn from_array(v: [f64; 4]) -> Self {
        Self::new(v[0], v[1], v[2], v[3])
    }

    pub fn to_array(self) -> [f64; 4] {
        [self.minx, self.miny, self.maxx, self.maxy]
    }

    /// Width in CRS units.
    pub fn width(&self) -> f64 {
        self.maxx - self.minx
    }

    /// Height in CRS units.
    pub fn height(&self) -> f64 {
        self.maxy - self.miny
    }

    /// True when the box is valid (min < max on both axes).
    pub fn is_valid(&self) -> bool {
        self.minx < self.maxx && self.miny < self.maxy
            && self.minx.is_finite()
            && self.miny.is_finite()
            && self.maxx.is_finite()
            && self.maxy.is_finite()
    }

    /// Inclusive intersection with another box; `None` when disjoint.
    pub fn intersect(&self, other: &Bbox) -> Option<Bbox> {
        let b = Bbox::new(
            self.minx.max(other.minx),
            self.miny.max(other.miny),
            self.maxx.min(other.maxx),
            self.maxy.min(other.maxy),
        );
        if b.is_valid() {
            Some(b)
        } else {
            None
        }
    }

    /// True when `other` is fully inside `self`.
    pub fn contains(&self, other: &Bbox) -> bool {
        other.minx >= self.minx
            && other.miny >= self.miny
            && other.maxx <= self.maxx
            && other.maxy <= self.maxy
    }

    /// Expand the box to include `other`.
    pub fn merge(&mut self, other: &Bbox) {
        self.minx = self.minx.min(other.minx);
        self.miny = self.miny.min(other.miny);
        self.maxx = self.maxx.max(other.maxx);
        self.maxy = self.maxy.max(other.maxy);
    }

    /// Parse `"minx,miny,maxx,maxy"`.
    pub fn parse(s: &str) -> Option<Bbox> {
        let v: Vec<f64> = s.split(',').map(|t| t.trim().parse::<f64>()).collect::<Result<_, _>>().ok()?;
        if v.len() != 4 {
            return None;
        }
        let b = Bbox::new(v[0], v[1], v[2], v[3]);
        b.is_valid().then_some(b)
    }
}
