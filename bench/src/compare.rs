use std::collections::HashMap;

fn load(path: &str) -> (Vec<String>, HashMap<String, f64>) {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    });
    let mut order = vec![];
    let mut values = HashMap::new();
    for line in text.lines().skip(1) {
        let mut cols = line.split(',');
        if let (Some(name), Some(v)) =
            (cols.next(), cols.next().and_then(|v| v.parse::<f64>().ok()))
        {
            order.push(name.to_string());
            values.insert(name.to_string(), v);
        }
    }
    (order, values)
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

pub fn run(args: &[String]) {
    let [before, after] = args else {
        eprintln!("usage: yawn-bench compare before.csv[,more.csv] after.csv[,more.csv]");
        std::process::exit(1);
    };
    let before: Vec<_> = before.split(',').map(load).collect();
    let after: Vec<_> = after.split(',').map(load).collect();
    println!("{:<28} {:>14} {:>14} {:>8}", "scenario", "before", "after", "ratio");
    let mut logs = vec![];
    for name in &before[0].0 {
        let pick = |set: &[(Vec<String>, HashMap<String, f64>)]| {
            let v: Vec<f64> = set.iter().filter_map(|(_, m)| m.get(name).copied()).collect();
            (!v.is_empty()).then(|| median(v))
        };
        let (Some(b), Some(a)) = (pick(&before), pick(&after)) else {
            continue;
        };
        let ratio = a / b;
        logs.push(ratio.ln());
        println!("{name:<28} {b:>14.1} {a:>14.1} {ratio:>8.2}");
    }
    if !logs.is_empty() {
        let geo = (logs.iter().sum::<f64>() / logs.len() as f64).exp();
        println!("geometric mean of ratios over {} rows: {geo:.3}", logs.len());
    }
}
