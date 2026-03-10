use crate::cfg::{FileCfg, FunctionCfg};
use serde::Serialize;
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
    serde_json::to_string_pretty(file_cfg)
        .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string())
}

#[derive(Serialize)]
struct AnalysisReport<'a> {
    files: &'a [FileCfg],
}

pub fn write_text_report(file_cfgs: &[FileCfg]) -> String {
    let mut out = String::new();
    let show_file_headers = file_cfgs.len() > 1;

    for (i, file_cfg) in file_cfgs.iter().enumerate() {
        if i > 0 {
            writeln!(out).unwrap();
        }
        if show_file_headers {
            writeln!(out, "# file: {}", file_cfg.file).unwrap();
            writeln!(out).unwrap();
        }
        write!(out, "{}", write_text(file_cfg)).unwrap();
    }

    out
}

pub fn write_json_report(file_cfgs: &[FileCfg]) -> String {
    serde_json::to_string_pretty(&AnalysisReport { files: file_cfgs })
        .unwrap_or_else(|e| serde_json::json!({"error": e.to_string()}).to_string())
}

fn escape_dot(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('{', "\\{")
        .replace('}', "\\}")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('|', "\\|")
}

/// Write DOT output for a FileCfg.
pub fn write_dot(file_cfg: &FileCfg) -> String {
    let mut out = String::new();
    writeln!(out, "digraph CFG {{").unwrap();
    writeln!(out, "    graph [rankdir=TB];").unwrap();
    writeln!(
        out,
        "    node [shape=record, fontname=\"Courier\", fontsize=10];"
    )
    .unwrap();
    writeln!(out).unwrap();

    for func in &file_cfg.functions {
        write_dot_function_with_prefix(&mut out, func, "", "    ");
    }

    writeln!(out, "}}").unwrap();
    out
}

pub fn write_dot_report(file_cfgs: &[FileCfg]) -> String {
    let mut out = String::new();
    writeln!(out, "digraph CFG {{").unwrap();
    writeln!(out, "    graph [rankdir=TB];").unwrap();
    writeln!(
        out,
        "    node [shape=record, fontname=\"Courier\", fontsize=10];"
    )
    .unwrap();
    writeln!(out).unwrap();

    let show_file_clusters = file_cfgs.len() > 1;
    for (file_idx, file_cfg) in file_cfgs.iter().enumerate() {
        if show_file_clusters {
            writeln!(out, "    subgraph cluster_file_{file_idx} {{").unwrap();
            writeln!(out, "        label=\"{}\";", escape_dot(&file_cfg.file)).unwrap();
            writeln!(out, "        style=dashed;").unwrap();
            writeln!(out).unwrap();
        }

        let indent = if show_file_clusters {
            "        "
        } else {
            "    "
        };
        for func in &file_cfg.functions {
            write_dot_function_with_prefix(&mut out, func, &format!("f{file_idx}_"), indent);
        }

        if show_file_clusters {
            writeln!(out, "    }}").unwrap();
            writeln!(out).unwrap();
        }
    }

    writeln!(out, "}}").unwrap();
    out
}

pub fn write_dot_function(out: &mut String, func: &FunctionCfg) {
    write_dot_function_with_prefix(out, func, "", "    ");
}

fn write_dot_function_with_prefix(
    out: &mut String,
    func: &FunctionCfg,
    namespace: &str,
    indent: &str,
) {
    let prefix = format!("{namespace}{}", func.name.replace('.', "_"));
    writeln!(out, "{indent}subgraph cluster_{prefix} {{").unwrap();
    writeln!(out, "{indent}    label=\"{}\";", escape_dot(&func.name)).unwrap();
    writeln!(out, "{indent}    style=rounded;").unwrap();
    writeln!(out).unwrap();

    for block in &func.blocks {
        let shape = if block.label == "entry" || block.label == "exit" {
            "Mrecord"
        } else {
            "record"
        };

        let mut label_parts = vec![format!("Block {} ({})", block.id, block.label)];
        for stmt in &block.statements {
            let escaped = escape_dot(&stmt.text);
            label_parts.push(format!("[L{}] {}", stmt.line, escaped));
        }
        let label = label_parts.join("\\l") + "\\l";

        writeln!(
            out,
            "{indent}    {prefix}_{id} [shape={shape}, label=\"{label}\"];",
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
                "{indent}    {prefix}_{src} -> {prefix}_{tgt} [label=\"{label}\", color={color}];",
                src = block.id,
                tgt = edge.target,
                label = escape_dot(&edge.label),
            )
            .unwrap();
        }
    }

    writeln!(out, "{indent}}}").unwrap();
    writeln!(out).unwrap();
}
