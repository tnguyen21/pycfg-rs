use crate::cfg::{FileCfg, FunctionCfg};
use std::fmt::Write;

/// Write text output for a FileCfg.
pub fn write_text(file_cfg: &FileCfg) -> String {
    let mut out = String::new();
    for (i, func) in file_cfg.functions.iter().enumerate() {
        if i > 0 {
            writeln!(out).unwrap();
        }
        write!(out, "{}", func).unwrap();
    }
    out
}

/// Write JSON output for a FileCfg.
pub fn write_json(file_cfg: &FileCfg) -> String {
    serde_json::to_string_pretty(file_cfg).unwrap_or_else(|e| format!("{{\"error\": \"{}\"}}", e))
}

/// Write DOT output for a FileCfg.
pub fn write_dot(file_cfg: &FileCfg) -> String {
    let mut out = String::new();
    writeln!(out, "digraph CFG {{").unwrap();
    writeln!(out, "    graph [rankdir=TB];").unwrap();
    writeln!(out, "    node [shape=record, fontname=\"Courier\", fontsize=10];").unwrap();
    writeln!(out).unwrap();

    for func in &file_cfg.functions {
        write_dot_function(&mut out, func);
    }

    writeln!(out, "}}").unwrap();
    out
}

fn write_dot_function(out: &mut String, func: &FunctionCfg) {
    let prefix = func.name.replace('.', "_");
    writeln!(out, "    subgraph cluster_{prefix} {{").unwrap();
    writeln!(out, "        label=\"{}\";", func.name).unwrap();
    writeln!(out, "        style=rounded;").unwrap();
    writeln!(out).unwrap();

    for block in &func.blocks {
        let shape = if block.label == "entry" || block.label == "exit" {
            "Mrecord"
        } else {
            "record"
        };

        let mut label_parts = vec![format!("Block {} ({})", block.id, block.label)];
        for stmt in &block.statements {
            // Escape special DOT characters
            let escaped = stmt
                .text
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('{', "\\{")
                .replace('}', "\\}")
                .replace('<', "\\<")
                .replace('>', "\\>")
                .replace('|', "\\|");
            label_parts.push(format!("[L{}] {}", stmt.line, escaped));
        }
        let label = label_parts.join("\\l") + "\\l";

        writeln!(
            out,
            "        {prefix}_{id} [shape={shape}, label=\"{label}\"];",
            id = block.id,
        )
        .unwrap();
    }

    writeln!(out).unwrap();

    // Edges
    for block in &func.blocks {
        for edge in &block.successors {
            let color = match edge.label.as_str() {
                "True" => "green",
                "False" => "red",
                "return" => "blue",
                "exception" | "raise" | "assert-fail" => "orange",
                "break" => "purple",
                "continue" => "cyan",
                _ => "black",
            };
            writeln!(
                out,
                "        {prefix}_{src} -> {prefix}_{tgt} [label=\"{label}\", color={color}];",
                src = block.id,
                tgt = edge.target,
                label = edge.label,
            )
            .unwrap();
        }
    }

    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}
