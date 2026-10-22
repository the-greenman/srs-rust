//! RFC-051 fractional keys: [R4] key validation and the Appendix's key algorithm
//! (fractional indexing after D. Greenspan, 2020). Pure and I/O-free.
//!
//! A key is an integer part (a head letter, then the digit count it names) followed by
//! an optional fraction that never ends in `0`. Digits are `0-9A-Za-z` (0..=61), so
//! byte order is digit order and keys compare byte-wise. `A` followed by 26 `0`s is
//! reserved and never written.

pub const CODE_KEY: &str = "arrangement-key";

const DIGITS: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// An [R4] violation or an unsatisfiable request; `code()` is always `arrangement-key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyError(pub String);

impl KeyError {
    pub fn code(&self) -> &'static str {
        CODE_KEY
    }
}

impl std::fmt::Display for KeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{CODE_KEY}: {}", self.0)
    }
}

impl std::error::Error for KeyError {}

fn err<T>(msg: impl Into<String>) -> Result<T, KeyError> {
    Err(KeyError(msg.into()))
}

fn digit(b: u8) -> Option<usize> {
    DIGITS.iter().position(|&d| d == b)
}

fn reserved_integer() -> String {
    format!("A{}", "0".repeat(26))
}

/// Length of the integer part (head included) that `head` names.
fn integer_len(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

/// [R4]: the key has the Appendix's shape and is not the reserved key.
pub fn validate_key(key: &str) -> Result<(), KeyError> {
    split(key).map(|_| ())
}

/// Splits a valid key into (integer part, fraction).
fn split(key: &str) -> Result<(&str, &str), KeyError> {
    let bytes = key.as_bytes();
    let Some(&head) = bytes.first() else {
        return err("a key must not be empty");
    };
    let Some(n) = integer_len(head) else {
        return err(format!("key {key:?} has no head letter"));
    };
    if bytes.len() < n {
        return err(format!(
            "key {key:?} is shorter than its integer part ({n})"
        ));
    }
    if let Some(b) = bytes[1..].iter().find(|&&b| digit(b).is_none()) {
        return err(format!("key {key:?} has the non-digit {:?}", *b as char));
    }
    if bytes.len() > n && bytes.last() == Some(&b'0') {
        return err(format!("key {key:?} has a fraction ending in 0"));
    }
    if key == reserved_integer() {
        return err("the key A followed by 26 0s is reserved");
    }
    Ok(key.split_at(n))
}

/// Next letter after/before `head`, or None at the end of the head range.
fn step_head(head: u8, up: bool) -> Option<u8> {
    match (head, up) {
        (b'z', true) | (b'A', false) => None,
        (b'Z', true) => Some(b'a'),
        (b'a', false) => Some(b'Z'),
        (h, true) => Some(h + 1),
        (h, false) => Some(h - 1),
    }
}

/// Adds one (`up`) or subtracts one to a valid integer part, keeping the head; on
/// overflow/underflow moves the head per the Appendix. None when there is no
/// successor (`z…z`) or predecessor (`A0…0`).
fn step_integer(int: &str, up: bool) -> Option<String> {
    let mut d: Vec<u8> = int.as_bytes().to_vec();
    let (wrap_from, wrap_to) = if up { (b'z', b'0') } else { (b'0', b'z') };
    for i in (1..d.len()).rev() {
        if d[i] == wrap_from {
            d[i] = wrap_to;
            continue;
        }
        let v = digit(d[i]).expect("validated digit");
        d[i] = DIGITS[if up { v + 1 } else { v - 1 }];
        return Some(String::from_utf8(d).expect("ascii"));
    }
    // Every digit wrapped: move the head, digits all wrap_to at the new length.
    let head = step_head(d[0], up)?;
    let len = integer_len(head).expect("letter head") - 1;
    Some(format!(
        "{}{}",
        head as char,
        (wrap_to as char).to_string().repeat(len)
    ))
}

/// The integer part of a valid key plus one; None when `z` has no successor.
pub fn increment_integer(int: &str) -> Result<Option<String>, KeyError> {
    check_integer(int)?;
    Ok(step_integer(int, true))
}

/// The integer part of a valid key minus one; None when `A` has no predecessor.
pub fn decrement_integer(int: &str) -> Result<Option<String>, KeyError> {
    check_integer(int)?;
    Ok(step_integer(int, false))
}

fn check_integer(int: &str) -> Result<(), KeyError> {
    let n = int.as_bytes().first().and_then(|&h| integer_len(h));
    if n != Some(int.len()) || int.bytes().skip(1).any(|b| digit(b).is_none()) {
        return err(format!("{int:?} is not an integer part"));
    }
    Ok(())
}

/// Midpoint of fractions `x < y` (`y` None is the end). Both are digit strings that
/// do not end in `0`; the result is strictly between them and does not end in `0`.
pub fn midpoint(x: &str, y: Option<&str>) -> Result<String, KeyError> {
    let ok = |s: &str| s.bytes().all(|b| digit(b).is_some()) && !s.ends_with('0');
    if !ok(x) || y.is_some_and(|y| !ok(y) || x >= y) {
        return err(format!("no midpoint between fractions {x:?} and {y:?}"));
    }
    Ok(mid(x.as_bytes(), y.map(str::as_bytes)))
}

fn mid(x: &[u8], y: Option<&[u8]>) -> String {
    if let Some(y) = y {
        let n = (0..y.len())
            .take_while(|&i| x.get(i).copied().unwrap_or(b'0') == y[i])
            .count();
        if n > 0 {
            let rest_x = x.get(n..).unwrap_or(&[]);
            let prefix = std::str::from_utf8(&y[..n]).expect("ascii");
            return format!("{prefix}{}", mid(rest_x, Some(&y[n..])));
        }
    }
    let dx = x.first().map_or(0, |&b| digit(b).expect("validated digit"));
    let dy = y.map_or(62, |y| digit(y[0]).expect("validated digit"));
    if dy - dx > 1 {
        return (DIGITS[(dx + dy).div_ceil(2)] as char).to_string();
    }
    match y {
        Some(y) if y.len() > 1 => (y[0] as char).to_string(),
        _ => format!(
            "{}{}",
            DIGITS[dx] as char,
            mid(x.get(1..).unwrap_or(&[]), None)
        ),
    }
}

/// The Appendix's key strictly between `a` and `b` (either absent means the open end).
/// Both present keys must be valid and `a < b`.
pub fn key_between(a: Option<&str>, b: Option<&str>) -> Result<String, KeyError> {
    let pa = a.map(split).transpose()?;
    let pb = b.map(split).transpose()?;
    if let (Some(a), Some(b)) = (a, b) {
        if a >= b {
            return err(format!("{a:?} is not less than {b:?}"));
        }
    }
    let reserved = reserved_integer();
    Ok(match (pa, pb) {
        (None, None) => "a0".to_string(),
        (None, Some((ib, fb))) => {
            if ib == reserved {
                format!("{ib}{}", mid(b"", Some(fb.as_bytes())))
            } else if !fb.is_empty() {
                ib.to_string()
            } else {
                match step_integer(ib, false) {
                    Some(d) if d != reserved => d,
                    _ => format!("{reserved}{}", mid(b"", None)),
                }
            }
        }
        (Some((ia, fa)), None) => match step_integer(ia, true) {
            Some(i) => i,
            None => format!("{ia}{}", mid(fa.as_bytes(), None)),
        },
        (Some((ia, fa)), Some((ib, fb))) if ia == ib => {
            format!("{ia}{}", mid(fa.as_bytes(), Some(fb.as_bytes())))
        }
        (Some((ia, fa)), Some(_)) => match step_integer(ia, true) {
            Some(i) if Some(i.as_str()) < b => i,
            _ => format!("{ia}{}", mid(fa.as_bytes(), None)),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kb(a: Option<&str>, b: Option<&str>) -> String {
        key_between(a, b).unwrap()
    }

    #[test]
    fn appendix_vectors() {
        assert_eq!(kb(None, None), "a0");
        assert_eq!(kb(Some("a0"), None), "a1");
        assert_eq!(kb(Some("az"), None), "b00");
        assert_eq!(kb(Some("a0"), Some("a1")), "a0V");
        assert_eq!(kb(None, Some("a0")), "Zz");
        let mut k = kb(None, None);
        for _ in 1..850 {
            k = kb(Some(&k), None);
        }
        assert_eq!(k, "bCh");
        let before = format!("A{}1", "0".repeat(25));
        assert_eq!(kb(None, Some(&before)), format!("A{}V", "0".repeat(26)));
    }

    #[test]
    fn integer_steps() {
        let inc = |s| increment_integer(s).unwrap();
        let dec = |s| decrement_integer(s).unwrap();
        assert_eq!(inc("Zz").as_deref(), Some("a0"));
        assert_eq!(inc("Yzz").as_deref(), Some("Z0"));
        assert_eq!(inc(&format!("z{}", "z".repeat(26))), None);
        assert_eq!(dec("a0").as_deref(), Some("Zz"));
        assert_eq!(dec("b00").as_deref(), Some("az"));
        assert_eq!(dec("Z0").as_deref(), Some("Yzz"));
        assert_eq!(dec(&reserved_integer()), None);
        assert_eq!(increment_integer("a").unwrap_err().code(), CODE_KEY);
    }

    #[test]
    fn rejects_invalid_keys() {
        for bad in ["", "!0", "a", "b0", "a0-", "a0V0", &reserved_integer()] {
            assert_eq!(validate_key(bad).unwrap_err().code(), CODE_KEY, "{bad:?}");
        }
        for good in [
            "a0",
            "Zz",
            "a0V",
            "b00",
            &format!("{}V", reserved_integer()),
        ] {
            validate_key(good).unwrap();
        }
        assert!(key_between(Some("a1"), Some("a0")).is_err());
        assert!(key_between(Some("a1"), Some("a1")).is_err());
        assert!(key_between(Some("a0V0"), None).is_err());
        assert!(midpoint("V", Some("1")).is_err());
    }

    /// Deterministic pseudo-random valid key (xorshift; no property crate in the
    /// workspace). Heads span both signs; fractions are 0..=3 digits.
    fn random_key(s: &mut u64) -> String {
        let mut next = |m: usize| {
            *s ^= *s << 13;
            *s ^= *s >> 7;
            *s ^= *s << 17;
            (*s % m as u64) as usize
        };
        loop {
            let head = if next(2) == 0 {
                b'a' + next(26) as u8
            } else {
                b'A' + next(26) as u8
            };
            let frac_len = next(4);
            // Bias digits towards the extremes so carries and wraps are exercised.
            let mut pick = || match next(4) {
                0 => b'0',
                1 => b'z',
                _ => DIGITS[next(62)],
            };
            let mut k = vec![head];
            for _ in 1..integer_len(head).unwrap() {
                k.push(pick());
            }
            for _ in 0..frac_len {
                k.push(pick());
            }
            let k = String::from_utf8(k).unwrap();
            if validate_key(&k).is_ok() {
                return k;
            }
        }
    }

    #[test]
    fn key_between_is_strictly_between() {
        let mut s = 0x9E37_79B9_7F4A_7C15u64;
        for _ in 0..20_000 {
            let (x, y) = (random_key(&mut s), random_key(&mut s));
            let (a, b) = if x < y { (x, y) } else { (y, x) };
            let cases = [
                (Some(a.as_str()), Some(b.as_str())),
                (Some(a.as_str()), None),
                (None, Some(b.as_str())),
            ];
            for (lo, hi) in cases {
                if lo.is_some() && lo == hi {
                    continue;
                }
                let k = key_between(lo, hi).unwrap();
                validate_key(&k).unwrap_or_else(|e| panic!("{lo:?} {hi:?} -> {k:?}: {e}"));
                assert!(lo.is_none_or(|lo| lo < k.as_str()), "{lo:?} < {k:?}");
                assert!(hi.is_none_or(|hi| k.as_str() < hi), "{k:?} < {hi:?}");
            }
        }
    }
}
