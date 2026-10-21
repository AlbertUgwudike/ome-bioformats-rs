//! Dimension orders and plane-index arithmetic.
//!
//! Port of `FormatTools.getIndex` / `FormatTools.getZCTCoords`.

use std::fmt;
use std::str::FromStr;

use crate::error::{Error, Result};

/// The rasterization order of the Z, C and T axes, fastest-varying first.
/// X and Y are always the two fastest axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DimensionOrder {
    #[default]
    XYZCT,
    XYZTC,
    XYCZT,
    XYCTZ,
    XYTZC,
    XYTCZ,
}

/// A plane's position along the Z, C (effective channel) and T axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Zct {
    pub z: u32,
    pub c: u32,
    pub t: u32,
}

impl DimensionOrder {
    pub const ALL: [DimensionOrder; 6] = [
        DimensionOrder::XYZCT,
        DimensionOrder::XYZTC,
        DimensionOrder::XYCZT,
        DimensionOrder::XYCTZ,
        DimensionOrder::XYTZC,
        DimensionOrder::XYTCZ,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            DimensionOrder::XYZCT => "XYZCT",
            DimensionOrder::XYZTC => "XYZTC",
            DimensionOrder::XYCZT => "XYCZT",
            DimensionOrder::XYCTZ => "XYCTZ",
            DimensionOrder::XYTZC => "XYTZC",
            DimensionOrder::XYTCZ => "XYTCZ",
        }
    }

    /// The three non-spatial axes, fastest-varying first.
    const fn axes(self) -> [u8; 3] {
        let s = self.as_str().as_bytes();
        [s[2], s[3], s[4]]
    }

    /// Converts ZCT coordinates to a plane index. `sizes` are
    /// `(sizeZ, effectiveSizeC, sizeT)`.
    pub fn index(self, sizes: (u32, u32, u32), zct: Zct) -> Result<u32> {
        let (sz, sc, st) = sizes;
        if zct.z >= sz || zct.c >= sc || zct.t >= st {
            return Err(Error::InvalidArgument(format!(
                "ZCT {zct:?} out of range for sizes Z={sz} C={sc} T={st}"
            )));
        }
        let pick = |axis: u8| match axis {
            b'Z' => (zct.z, sz),
            b'C' => (zct.c, sc),
            _ => (zct.t, st),
        };
        let [a0, a1, a2] = self.axes();
        let (v0, l0) = pick(a0);
        let (v1, l1) = pick(a1);
        let (v2, _) = pick(a2);
        Ok(v0 + v1 * l0 + v2 * l0 * l1)
    }

    /// Converts a plane index to ZCT coordinates.
    pub fn zct(self, sizes: (u32, u32, u32), index: u32) -> Result<Zct> {
        let (sz, sc, st) = sizes;
        let count = sz * sc * st;
        if index >= count {
            return Err(Error::InvalidArgument(format!(
                "plane {index} out of range (image count {count})"
            )));
        }
        let len = |axis: u8| match axis {
            b'Z' => sz,
            b'C' => sc,
            _ => st,
        };
        let [a0, a1, a2] = self.axes();
        let (l0, l1) = (len(a0), len(a1));
        let v0 = index % l0;
        let v1 = index / l0 % l1;
        let v2 = index / l0 / l1;
        let mut out = Zct::default();
        for (axis, v) in [(a0, v0), (a1, v1), (a2, v2)] {
            match axis {
                b'Z' => out.z = v,
                b'C' => out.c = v,
                _ => out.t = v,
            }
        }
        Ok(out)
    }
}

impl fmt::Display for DimensionOrder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for DimensionOrder {
    type Err = Error;

    /// Accepts the six OME orders. Also accepts `YX...` orders, which some
    /// readers produce for transposed storage, by treating them as `XY...`.
    fn from_str(s: &str) -> Result<Self> {
        let upper = s.to_ascii_uppercase();
        let normalized = upper
            .strip_prefix("YX")
            .map(|rest| format!("XY{rest}"))
            .unwrap_or(upper);
        Self::ALL
            .into_iter()
            .find(|o| o.as_str() == normalized)
            .ok_or_else(|| Error::Format(format!("invalid dimension order {s:?}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_round_trips_for_every_order() {
        let sizes = (3, 2, 4);
        for order in DimensionOrder::ALL {
            for i in 0..24 {
                let zct = order.zct(sizes, i).unwrap();
                assert_eq!(order.index(sizes, zct).unwrap(), i, "{order}");
            }
        }
    }

    #[test]
    fn xyczt_layout() {
        let o = DimensionOrder::XYCZT;
        assert_eq!(o.zct((2, 3, 1), 4).unwrap(), Zct { z: 1, c: 1, t: 0 });
    }
}
