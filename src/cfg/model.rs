use serde::{Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BlockKind {
    Entry,
    Exit,
    Body,
}

impl BlockKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlockKind::Entry => "entry",
            BlockKind::Exit => "exit",
            BlockKind::Body => "body",
        }
    }
}

impl From<&str> for BlockKind {
    fn from(value: &str) -> Self {
        match value {
            "entry" => BlockKind::Entry,
            "exit" => BlockKind::Exit,
            "body" => BlockKind::Body,
            other => panic!("unsupported block kind: {other}"),
        }
    }
}

impl Serialize for BlockKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl fmt::Display for BlockKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for BlockKind {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EdgeKind {
    True,
    False,
    Return,
    Exception,
    Raise,
    AssertFail,
    Break,
    Continue,
    LoopBody,
    LoopExit,
    LoopBack,
    Fallthrough,
    Try,
    TryElse,
    Finally,
    Case(String),
    Other(String),
}

impl EdgeKind {
    pub fn as_str(&self) -> &str {
        match self {
            EdgeKind::True => "True",
            EdgeKind::False => "False",
            EdgeKind::Return => "return",
            EdgeKind::Exception => "exception",
            EdgeKind::Raise => "raise",
            EdgeKind::AssertFail => "assert-fail",
            EdgeKind::Break => "break",
            EdgeKind::Continue => "continue",
            EdgeKind::LoopBody => "loop-body",
            EdgeKind::LoopExit => "loop-exit",
            EdgeKind::LoopBack => "loop-back",
            EdgeKind::Fallthrough => "fallthrough",
            EdgeKind::Try => "try",
            EdgeKind::TryElse => "try-else",
            EdgeKind::Finally => "finally",
            EdgeKind::Case(label) | EdgeKind::Other(label) => label.as_str(),
        }
    }

    pub fn starts_with(&self, prefix: &str) -> bool {
        self.as_str().starts_with(prefix)
    }

    pub fn dot_color(&self) -> &'static str {
        match self {
            EdgeKind::True => "green",
            EdgeKind::False => "red",
            EdgeKind::Return => "blue",
            EdgeKind::Exception | EdgeKind::Raise | EdgeKind::AssertFail => "orange",
            EdgeKind::Break => "purple",
            EdgeKind::Continue => "cyan",
            _ => "black",
        }
    }
}

impl From<&str> for EdgeKind {
    fn from(value: &str) -> Self {
        match value {
            "True" => EdgeKind::True,
            "False" => EdgeKind::False,
            "return" => EdgeKind::Return,
            "exception" => EdgeKind::Exception,
            "raise" => EdgeKind::Raise,
            "assert-fail" => EdgeKind::AssertFail,
            "break" => EdgeKind::Break,
            "continue" => EdgeKind::Continue,
            "loop-body" => EdgeKind::LoopBody,
            "loop-exit" => EdgeKind::LoopExit,
            "loop-back" => EdgeKind::LoopBack,
            "fallthrough" => EdgeKind::Fallthrough,
            "try" => EdgeKind::Try,
            "try-else" => EdgeKind::TryElse,
            "finally" => EdgeKind::Finally,
            label if label.starts_with("case ") => EdgeKind::Case(label.to_string()),
            label => EdgeKind::Other(label.to_string()),
        }
    }
}

impl From<String> for EdgeKind {
    fn from(value: String) -> Self {
        EdgeKind::from(value.as_str())
    }
}

impl Serialize for EdgeKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for EdgeKind {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub target: usize,
    pub label: EdgeKind,
}

#[derive(Debug, Clone, Serialize)]
pub struct Statement {
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BasicBlock {
    pub id: usize,
    pub label: BlockKind,
    pub statements: Vec<Statement>,
    pub successors: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Metrics {
    pub blocks: usize,
    pub edges: usize,
    pub branches: usize,
    pub cyclomatic_complexity: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionCfg {
    pub name: String,
    pub line: usize,
    pub blocks: Vec<BasicBlock>,
    pub metrics: Metrics,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileCfg {
    pub file: String,
    pub functions: Vec<FunctionCfg>,
}

impl Metrics {
    pub(crate) fn compute(blocks: &[BasicBlock]) -> Self {
        let num_blocks = blocks.len();
        let num_edges: usize = blocks.iter().map(|b| b.successors.len()).sum();
        let branches = blocks.iter().filter(|b| b.successors.len() > 1).count();
        let cyclomatic = if num_blocks == 0 {
            1
        } else {
            (num_edges as isize - num_blocks as isize + 2).max(1) as usize
        };
        Metrics {
            blocks: num_blocks,
            edges: num_edges,
            branches,
            cyclomatic_complexity: cyclomatic,
        }
    }
}

impl fmt::Display for FunctionCfg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "def {}:", self.name)?;
        writeln!(f)?;
        for block in &self.blocks {
            if block.label == "entry" || block.label == "exit" {
                write!(f, "  Block {} ({}):", block.id, block.label)?;
            } else {
                write!(f, "  Block {}:", block.id)?;
            }
            writeln!(f)?;
            for stmt in &block.statements {
                writeln!(f, "    [L{}] {}", stmt.line, stmt.text)?;
            }
            for edge in &block.successors {
                writeln!(f, "    -> Block {} [{}]", edge.target, edge.label)?;
            }
            writeln!(f)?;
        }
        writeln!(
            f,
            "  # blocks={} edges={} branches={} cyclomatic_complexity={}",
            self.metrics.blocks,
            self.metrics.edges,
            self.metrics.branches,
            self.metrics.cyclomatic_complexity
        )
    }
}
