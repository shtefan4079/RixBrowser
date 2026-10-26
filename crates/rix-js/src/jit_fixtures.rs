
pub const NUMBER_FINGERPRINT_SOURCE: &str = r#"function numberFingerprint(r) {
    if (r !== r) return "NaN";
    if (r === 0) return (1 / r < 0) ? "-0" : "0";
    if (r === Infinity) return "Infinity";
    if (r === -Infinity) return "-Infinity";
    return String(r);
}
"#;

pub struct Fixture {
    pub name: &'static str,
    pub description: &'static str,
    body: &'static str,
}

impl Fixture {
    #[must_use]
    pub fn source(&self, iterations: u64) -> String {
        let mut source =
            String::with_capacity(NUMBER_FINGERPRINT_SOURCE.len() + self.body.len() + 24);
        source.push_str(NUMBER_FINGERPRINT_SOURCE);
        source.push_str(&self.body.replace("{iterations}", &iterations.to_string()));
        source
    }
}

pub const FIXTURES: &[Fixture] = &[
    Fixture {
        name: "mix",
        description: "integer arithmetic in a while loop (the original workload)",
        body: r#"function mix(a, b) {
    let t = a;
    let i = 0;
    while (i < 200) {
        t = t + i * b;
        if (t >= 1000003) {
            t = t - 1000003;
        }
        i = i + 1;
    }
    return t;
}
let acc = 0;
let k = 0;
while (k < {iterations}) {
    acc = acc + mix(k, 3);
    if (acc >= 1000003) {
        acc = acc - 1000003;
    }
    k = k + 1;
}
numberFingerprint(acc);
"#,
    },
    Fixture {
        name: "props",
        description: "object property reads and writes inside a loop",
        body: r#"function accumulate(seed) {
    const cell = { value: seed, parity: 0, count: 0 };
    let i = 0;
    while (i < 16) {
        cell.value = cell.value + i;
        if ((cell.value & 1) === 0) {
            cell.parity = cell.parity + 1;
        }
        cell.count = cell.count + 1;
        i = i + 1;
    }
    return cell.value + cell.parity * 10 + cell.count;
}
let acc = 0;
let k = 0;
while (k < {iterations}) {
    acc = acc + accumulate(k);
    if (acc >= 1000003) {
        acc = acc - 1000003;
    }
    k = k + 1;
}
numberFingerprint(acc);
"#,
    },
    Fixture {
        name: "calls",
        description: "one function call per loop iteration",
        body: r#"function step(x) {
    return x * 3 + 1;
}
function drive(n) {
    let t = 0;
    let i = 0;
    while (i < n) {
        t = t + step(i);
        if (t >= 1000003) {
            t = t - 1000003;
        }
        i = i + 1;
    }
    return t;
}
let acc = 0;
let k = 0;
while (k < {iterations}) {
    acc = acc + drive(32);
    if (acc >= 1000003) {
        acc = acc - 1000003;
    }
    k = k + 1;
}
numberFingerprint(acc);
"#,
    },
    Fixture {
        name: "text",
        description: "array indexing, string indexing and concatenation",
        body: r#"function walk(values, text, n) {
    let out = "";
    let sum = 0;
    let i = 0;
    while (i < n) {
        const v = values[i % values.length];
        out = out + text[v % text.length];
        if (out.length >= 16) {
            out = "abcdefgh";
        }
        sum = sum + out.length;
        i = i + 1;
    }
    return sum * 10 + out.length;
}
const values = [0, 1, 2, 3, 4, 5, 6, 7];
const text = "abcdefghijklmnop";
let acc = 0;
let k = 0;
while (k < {iterations}) {
    acc = acc + walk(values, text, 16);
    if (acc >= 1000003) {
        acc = acc - 1000003;
    }
    k = k + 1;
}
numberFingerprint(acc);
"#,
    },
    Fixture {
        name: "control",
        description: "nested loops, switch, break/continue and early exits",
        body: r#"function classify(n) {
    let t = n;
    let i = 0;
    outer: while (i < 8) {
        switch (i % 4) {
            case 0: t = t + i * 3; break;
            case 1: t = t + i * 5; break;
            case 2: i = i + 1; continue;
            default: t = t + 7;
        }
        let j = 0;
        while (j < 8) {
            j = j + 1;
            if ((i + j) % 5 === 0) {
                continue;
            }
            if (j > 6) {
                break;
            }
            t = t + j;
            if (t > 1000) {
                break outer;
            }
        }
        i = i + 1;
    }
    return t;
}
let acc = 0;
let k = 0;
while (k < {iterations}) {
    acc = acc + classify(k);
    if (acc >= 1000003) {
        acc = acc - 1000003;
    }
    k = k + 1;
}
numberFingerprint(acc);
"#,
    },
];

#[must_use]
pub fn fixture(name: &str) -> Option<&'static Fixture> {
    FIXTURES.iter().find(|fixture| fixture.name == name)
}
