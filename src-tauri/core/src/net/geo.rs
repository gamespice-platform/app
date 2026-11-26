//! Distance-based latency estimates from a local IP geolocation database.
//!
//! This is the app's half of `EstimatePingMs` in the website
//! (`website-city/src/utils/location.ts`) and MUST give the same number for the
//! same two points: haversine distance on a 6371 km sphere, 0.02 ms per km
//! (round trip in fibre), a 5 ms floor, then a 1.3 routing fudge, rounded. The
//! website gets its two points from the server row and the browser's
//! geolocation prompt; the app has neither, so it looks both IP addresses up in
//! an IP2Location LITE database held on this machine ([`GeoDb`]).
//!
//! **An estimate, never a measurement.** It says how far away the machine
//! probably is, not what the route costs, and the UI labels it so (`~45 ms
//! est.`). The measured figure is `net::ping`.
//!
//! # The on-disk database
//!
//! IP2Location's CSV is ~100 MB for IPv4 + IPv6. Latency needs only a point per
//! range, so [`GeoDb::from_csv`] keeps `(range start, lat, lon)` and drops
//! everything else, with coordinates quantised to 0.01 degree (about 1 km,
//! far below the error of the estimate itself):
//!
//! | Part | Layout (little endian) |
//! | --- | --- |
//! | header | `GSGEO001`, built-at unix seconds `u64`, IPv4 count `u32`, IPv6 count `u32` |
//! | IPv4 | per range: start `u32`, lat `i16`, lon `i16` (8 bytes) |
//! | IPv6 | per range: start's top 64 bits `u64`, lat `i16`, lon `i16` (12 bytes) |
//!
//! A range's end is the next range's start (IP2Location's table is gap-free;
//! its "unknown" rows are stored with latitude `i16::MIN` so a lookup that lands
//! in one answers `None` rather than the Gulf of Guinea).

use std::io::BufRead;
use std::net::IpAddr;

use crate::error::{AppError, AppResult};

const MAGIC: &[u8; 8] = b"GSGEO001";
const HEADER: usize = 8 + 8 + 4 + 4;
const REC4: usize = 8;
const REC6: usize = 12;
/// Coordinates are stored as hundredths of a degree.
const SCALE: f64 = 100.0;
const UNKNOWN: i16 = i16::MIN;
/// `::ffff:0:0`, where the IPv6 CSV puts the IPv4 space.
const V4_MAPPED: u128 = 0xffff_0000_0000;

/// The website's `EstimatePingMs`, point for point. See the module header.
pub fn estimate_ping_ms(client_lat: f64, client_lon: f64, server_lat: f64, server_lon: f64) -> u32 {
    let to_rad = |deg: f64| deg * std::f64::consts::PI / 180.0;
    let d_lat = to_rad(server_lat - client_lat);
    let d_lon = to_rad(server_lon - client_lon);
    let a = (d_lat / 2.0).sin().powi(2)
        + to_rad(client_lat).cos() * to_rad(server_lat).cos() * (d_lon / 2.0).sin().powi(2);
    let distance_km = 6371.0 * 2.0 * a.sqrt().atan2((1.0 - a).sqrt());

    // Speed of light in fibre is ~200,000 km/s: ~0.02 ms per km round trip.
    let ms = (distance_km * 0.02).max(5.0) * 1.3;

    ms.round() as u32
}

/// A loaded database. Cheap to query, ~8-12 bytes per range in memory.
pub struct GeoDb {
    bytes: Vec<u8>,
    n4: usize,
    n6: usize,
    built_at: u64,
}

impl GeoDb {
    /// Read the compact format back, refusing anything that is not exactly it.
    pub fn from_bytes(bytes: Vec<u8>) -> AppResult<Self> {
        if bytes.len() < HEADER || &bytes[..8] != MAGIC {
            return Err(AppError::invalid(
                "The geolocation database file is not in this app's format; delete it and download it again.",
            ));
        }

        let built_at = u64::from_le_bytes(bytes[8..16].try_into().unwrap());
        let n4 = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
        let n6 = u32::from_le_bytes(bytes[20..24].try_into().unwrap()) as usize;

        if bytes.len() != HEADER + n4 * REC4 + n6 * REC6 {
            return Err(AppError::invalid(
                "The geolocation database file is truncated; delete it and download it again.",
            ));
        }

        Ok(Self {
            bytes,
            n4,
            n6,
            built_at,
        })
    }

    pub fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn built_at(&self) -> u64 {
        self.built_at
    }

    pub fn counts(&self) -> (usize, usize) {
        (self.n4, self.n6)
    }

    /// Convert IP2Location LITE CSV text (any DB with lat/lon at columns 6 and
    /// 7: DB5, DB9, DB11, ...; the plain or the IPv6 file) into the compact
    /// format. Strict: a malformed line is an error naming it, because a
    /// half-read database would estimate wrongly without anyone noticing.
    pub fn from_csv(reader: impl BufRead, built_at: u64) -> AppResult<Self> {
        let mut v4: Vec<(u32, i16, i16)> = Vec::new();
        let mut v6: Vec<(u64, i16, i16)> = Vec::new();

        for (n, line) in reader.lines().enumerate() {
            let line =
                line.map_err(|e| AppError::invalid(format!("Reading the CSV failed: {e}")))?;
            let line = line.trim();

            if line.is_empty() {
                continue;
            }

            let bad = |why: &str| AppError::invalid(format!("CSV line {}: {why}.", n + 1));
            let fields: Vec<&str> = line.trim_matches('"').split("\",\"").collect();

            if fields.len() < 8 {
                return Err(bad("expected at least 8 columns"));
            }

            let from: u128 = fields[0]
                .parse()
                .map_err(|_| bad("range start is not a number"))?;
            let lat: f64 = fields[6]
                .parse()
                .map_err(|_| bad("latitude is not a number"))?;
            let lon: f64 = fields[7]
                .parse()
                .map_err(|_| bad("longitude is not a number"))?;

            if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                return Err(bad("coordinates out of range"));
            }

            // IP2Location writes 0,0 for "no location".
            let (qlat, qlon) = if lat == 0.0 && lon == 0.0 {
                (UNKNOWN, 0)
            } else {
                ((lat * SCALE).round() as i16, (lon * SCALE).round() as i16)
            };

            if from < (1u128 << 32) {
                v4.push((from as u32, qlat, qlon));
            } else if (V4_MAPPED..V4_MAPPED + (1u128 << 32)).contains(&from) {
                v4.push(((from - V4_MAPPED) as u32, qlat, qlon));
            } else {
                v6.push(((from >> 64) as u64, qlat, qlon));
            }
        }

        if v4.is_empty() && v6.is_empty() {
            return Err(AppError::invalid("The CSV held no address ranges."));
        }

        v4.sort_by_key(|r| r.0);
        v6.sort_by_key(|r| r.0);

        let mut bytes = Vec::with_capacity(HEADER + v4.len() * REC4 + v6.len() * REC6);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&built_at.to_le_bytes());
        bytes.extend_from_slice(&(v4.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&(v6.len() as u32).to_le_bytes());

        for (from, lat, lon) in &v4 {
            bytes.extend_from_slice(&from.to_le_bytes());
            bytes.extend_from_slice(&lat.to_le_bytes());
            bytes.extend_from_slice(&lon.to_le_bytes());
        }

        for (from, lat, lon) in &v6 {
            bytes.extend_from_slice(&from.to_le_bytes());
            bytes.extend_from_slice(&lat.to_le_bytes());
            bytes.extend_from_slice(&lon.to_le_bytes());
        }

        Self::from_bytes(bytes)
    }

    /// Where an address is, as (latitude, longitude), or `None` when the
    /// database does not know.
    pub fn lookup(&self, ip: IpAddr) -> Option<(f64, f64)> {
        let ip = match ip {
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => IpAddr::V4(v4),
                None => IpAddr::V6(v6),
            },
            v4 => v4,
        };

        let (off, count, rec, key): (usize, usize, usize, u64) = match ip {
            IpAddr::V4(a) => (HEADER, self.n4, REC4, u64::from(u32::from(a))),
            IpAddr::V6(a) => (
                HEADER + self.n4 * REC4,
                self.n6,
                REC6,
                (u128::from(a) >> 64) as u64,
            ),
        };

        let start = |i: usize| -> u64 {
            let p = off + i * rec;
            match rec {
                REC4 => u64::from(u32::from_le_bytes(self.bytes[p..p + 4].try_into().unwrap())),
                _ => u64::from_le_bytes(self.bytes[p..p + 8].try_into().unwrap()),
            }
        };

        // The last range that starts at or before the address.
        let (mut lo, mut hi) = (0usize, count);

        while lo < hi {
            let mid = (lo + hi) / 2;

            if start(mid) <= key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }

        let i = lo.checked_sub(1)?;
        let p = off + i * rec + rec - 4;
        let lat = i16::from_le_bytes(self.bytes[p..p + 2].try_into().unwrap());
        let lon = i16::from_le_bytes(self.bytes[p + 2..p + 4].try_into().unwrap());

        (lat != UNKNOWN).then(|| (f64::from(lat) / SCALE, f64::from(lon) / SCALE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CSV: &str = "\
\"0\",\"16777215\",\"-\",\"-\",\"-\",\"-\",\"0.000000\",\"0.000000\"
\"16777216\",\"16777471\",\"AU\",\"Australia\",\"Queensland\",\"South Brisbane\",\"-27.474800\",\"153.017100\"
\"16777472\",\"16778239\",\"CN\",\"China\",\"Fujian\",\"Fuzhou\",\"26.061390\",\"119.306110\"
\"134744064\",\"134744319\",\"US\",\"United States\",\"California\",\"Mountain View\",\"37.405992\",\"-122.078515\"
\"281470833330432\",\"281470833330687\",\"DE\",\"Germany\",\"Hessen\",\"Frankfurt\",\"50.110920\",\"8.682130\"
\"42541956101370907050197289607612071936\",\"42541956180599069564461627201156022271\",\"US\",\"United States\",\"California\",\"Los Angeles\",\"34.052230\",\"-118.243680\"
";

    fn db() -> GeoDb {
        GeoDb::from_csv(CSV.as_bytes(), 1_700_000_000).unwrap()
    }

    #[test]
    fn looks_up_v4_ranges_and_the_gap_before_the_first() {
        let db = db();
        let (lat, lon) = db.lookup("1.0.0.10".parse().unwrap()).unwrap();
        assert!((lat + 27.47).abs() < 0.01 && (lon - 153.02).abs() < 0.01);
        let (lat, _) = db.lookup("1.0.5.1".parse().unwrap()).unwrap();
        assert!((lat - 26.06).abs() < 0.01);
        assert_eq!(db.lookup("0.1.2.3".parse().unwrap()), None, "unknown row");
        let (lat, lon) = db.lookup("8.8.8.8".parse().unwrap()).unwrap();
        assert!((lat - 37.41).abs() < 0.01 && (lon + 122.08).abs() < 0.01);
    }

    #[test]
    fn ipv4_mapped_rows_and_addresses_share_the_v4_table() {
        let db = db();
        // The Frankfurt row is written as ::ffff:9.9.9.0 in the IPv6 CSV.
        let (lat, _) = db.lookup("9.9.9.9".parse().unwrap()).unwrap();
        assert!((lat - 50.11).abs() < 0.01);
        assert_eq!(
            db.lookup("::ffff:9.9.9.9".parse().unwrap()),
            db.lookup("9.9.9.9".parse().unwrap())
        );
        assert_eq!(db.counts(), (5, 1));
    }

    #[test]
    fn looks_up_v6() {
        let db = db();
        let (lat, _) = db.lookup("2001:4860:4860::8888".parse().unwrap()).unwrap();
        assert!((lat - 34.05).abs() < 0.01);
    }

    #[test]
    fn survives_a_round_trip_through_bytes() {
        let db = db();
        let again = GeoDb::from_bytes(db.to_bytes().to_vec()).unwrap();
        assert_eq!(again.built_at(), 1_700_000_000);
        assert_eq!(again.counts(), (5, 1));
    }

    #[test]
    fn refuses_truncated_or_foreign_files() {
        assert!(GeoDb::from_bytes(b"nonsense".to_vec()).is_err());
        let mut bytes = db().to_bytes().to_vec();
        bytes.truncate(bytes.len() - 3);
        assert!(GeoDb::from_bytes(bytes).is_err());
    }

    #[test]
    fn a_malformed_line_is_an_error_naming_it() {
        let err = GeoDb::from_csv("\"1\",\"2\",\"x\"\n".as_bytes(), 0)
            .err()
            .unwrap();
        assert!(err.to_string().contains("line 1"), "{err}");
        assert!(GeoDb::from_csv("".as_bytes(), 0).is_err());
    }

    /// The same vectors the website's `EstimatePingMs` returns (see
    /// `website-city/src/utils/location.ts`): that one is the reference.
    #[test]
    fn distance_to_latency_matches_the_websites_formula() {
        // Same point: the 5 ms floor, times 1.3, is 6.5 -> 7 (JS Math.round(6.5) = 7).
        assert_eq!(estimate_ping_ms(50.0, 8.0, 50.0, 8.0), 7);
        // London -> New York, 5570 km: 5570*0.02*1.3 = 144.8.
        let ms = estimate_ping_ms(51.5074, -0.1278, 40.7128, -74.0060);
        assert!((144..=146).contains(&ms), "{ms}");
        // Sydney -> Frankfurt, ~16,500 km: ~430 ms.
        let ms = estimate_ping_ms(-33.8688, 151.2093, 50.1109, 8.6821);
        assert!((425..=435).contains(&ms), "{ms}");
        // Symmetric, and farther is never faster.
        assert_eq!(
            estimate_ping_ms(10.0, 20.0, 30.0, 40.0),
            estimate_ping_ms(30.0, 40.0, 10.0, 20.0)
        );
        assert!(estimate_ping_ms(0.0, 0.0, 0.0, 90.0) > estimate_ping_ms(0.0, 0.0, 0.0, 10.0));
    }
}
