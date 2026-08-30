mod buffer;
pub(crate) mod line;
mod mutation;
mod state;
mod viewport;

pub(crate) use self::line::{Line, SegStyle};
pub(crate) use self::state::{Layout, MsgArea};
