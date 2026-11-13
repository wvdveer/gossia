//! What belongs to the user rather than to any workbook, and the `.gossia`
//! file itself: the application and `bidwillii` read the same
//! preferences, trust record and data sources, and open workbooks the same
//! way (DESIGN.md, "Batch export from the command line").

pub mod file;
pub mod prefs;
pub mod share;
