//! Importer for Visio `.vsdx` drawings.
//!
//! A drawing is an Open Packaging Conventions ZIP holding XML parts: a page
//! index, one part per page, and one per stencil master. Microsoft publishes
//! the layout as `[MS-VSDX]`, and this importer was written from that
//! specification.
//!
//! # What is imported
//!
//! Pages, their sizes, and their names. Shape position, size, and rotation.
//! Shape text. Fill color, line color, line weight, and dash pattern. Masters,
//! mapped onto the shape types this build draws. Connectors, including which
//! shapes each end attaches to. Groups, flattened to their member shapes.
//!
//! # Coordinates
//!
//! Visio measures in inches from the **bottom left** of the page with the y
//! axis pointing up, and positions a shape by its *pin* — the point it rotates
//! about — rather than by a corner. This module is where that is converted to
//! the top-left origin, downward y, and corner-anchored boxes the rest of
//! DiagramCraft uses; nothing downstream needs to know Visio's convention.

use dc_model::{
    Document, Endpoint, EndpointStyle, Line, Page, Shape, ShapeType, Text, Units,
};
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek};
use std::path::Path;

pub mod masters;
pub mod write;

pub use write::{WriteError, write as write_vsdx, write_path as write_vsdx_path};
pub mod package;
pub mod sheet;
pub mod xml;

pub use masters::shape_type_for;
pub use sheet::Sheet;

use package::Package;
use xml::Node;

/// Points per inch, which is the unit Visio stores lengths in.
const POINTS_PER_INCH: f64 = 72.0;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("not a valid zip archive: {0}")]
    Zip(#[from] zip::result::ZipError),

    #[error("malformed xml: {0}")]
    Xml(String),

    #[error("package has no visio/pages/pages.xml; it may not be a .vsdx file")]
    NotAVisioDrawing,
}

/// A drawing after import, with anything the importer could not carry over.
#[derive(Debug)]
pub struct Import {
    pub document: Document,
    /// Masters the importer did not recognize, and anything else it had to
    /// approximate. Shown to the user rather than hidden.
    pub warnings: Vec<String>,
}

impl Import {
    /// A one-line summary suitable for a status bar.
    pub fn summary(&self) -> String {
        let pages = self.document.pages.len();
        let shapes: usize = self.document.pages.iter().map(|p| p.shapes().len()).sum();
        let lines: usize = self.document.pages.iter().map(|p| p.lines().len()).sum();
        let base = format!("Imported {pages} page(s), {shapes} shape(s), {lines} connector(s)");
        if self.warnings.is_empty() {
            base
        } else {
            format!("{base}; {} not fully supported", self.warnings.len())
        }
    }
}

/// Reads a `.vsdx` drawing from a path.
pub fn read_path(path: impl AsRef<Path>) -> Result<Import, Error> {
    let file = std::fs::File::open(path)?;
    read(file)
}

/// Reads a `.vsdx` drawing from any seekable reader.
pub fn read<R: Read + Seek>(reader: R) -> Result<Import, Error> {
    let package = Package::read(reader)?;
    Importer::new(package).run()
}

/// Whether a path looks like a Visio drawing this importer handles.
pub fn is_vsdx_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("vsdx") || e.eq_ignore_ascii_case("vsdm"))
}

struct Importer {
    package: Package,
    warnings: Vec<String>,
    /// Master id to its name and the cells it supplies to shapes using it.
    masters: BTreeMap<String, MasterInfo>,
    /// Ids already handed out, so every page's shapes stay globally unique.
    used_ids: std::collections::HashSet<String>,
    next_id: u32,
}

#[derive(Clone, Debug, Default)]
struct MasterInfo {
    name: String,
    cells: BTreeMap<String, String>,
}

impl Importer {
    fn new(package: Package) -> Self {
        Importer {
            package,
            warnings: Vec::new(),
            masters: BTreeMap::new(),
            used_ids: std::collections::HashSet::new(),
            next_id: 1,
        }
    }

    fn run(mut self) -> Result<Import, Error> {
        if !self.package.has("visio/pages/pages.xml") {
            return Err(Error::NotAVisioDrawing);
        }

        self.load_masters();

        let mut document = Document::new();
        // Visio stores inches; keeping the document in points means every
        // length in the model is already in the unit the renderer draws in.
        document.set_units(Units::Pt);
        document.pages.clear();

        let pages_xml = self
            .package
            .text("visio/pages/pages.xml")
            .ok_or(Error::NotAVisioDrawing)?;
        let pages = xml::parse(&pages_xml)?;
        let relationships = self.relationships_of("visio/pages/pages.xml");

        for (index, page_node) in pages.descendants("Page").into_iter().rev().enumerate() {
            // A background page is referenced by the page it sits behind and
            // has no tab of its own.
            if page_node.attr("Background").is_some_and(is_true) {
                continue;
            }
            let page = self.import_page(index, page_node, &relationships)?;
            document.pages.push(page);
        }

        if document.pages.is_empty() {
            self.warnings
                .push("the drawing declared no foreground pages".into());
            document.pages.push(Page::new("page1", "Page 1"));
        }

        Ok(Import { document, warnings: self.warnings })
    }

    /// Reads every master's name and the cells it lends to its instances.
    fn load_masters(&mut self) {
        let Some(masters_xml) = self.package.text("visio/masters/masters.xml") else {
            return;
        };
        let Ok(masters) = xml::parse(&masters_xml) else {
            self.warnings.push("the stencil index could not be read".into());
            return;
        };
        let relationships = self.relationships_of("visio/masters/masters.xml");

        for master in masters.descendants("Master") {
            let Some(id) = master.attr("ID") else { continue };
            let name = master
                .attr("NameU")
                .or_else(|| master.attr("Name"))
                .unwrap_or_default()
                .to_string();

            // The master's own shape supplies defaults for anything an
            // instance leaves unset, which is most of a stock shape.
            let mut cells = BTreeMap::new();
            if let Some(target) = master
                .child("Rel")
                .and_then(|r| r.attr("id"))
                .and_then(|id| relationships.get(id))
            {
                if let Some(text) = self.package.text(target) {
                    if let Ok(contents) = xml::parse(&text) {
                        if let Some(shape) = contents.descendants("Shape").into_iter().next_back() {
                            cells = sheet::cells_of(shape);
                        }
                    }
                }
            }

            self.masters.insert(id.to_string(), MasterInfo { name, cells });
        }
    }

    /// The relationship id to resolved part path map for a part.
    fn relationships_of(&self, part: &str) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let path = package::rels_path(part);
        let Some(text) = self.package.text(&path) else {
            return map;
        };
        let Ok(root) = xml::parse(&text) else {
            return map;
        };
        for relationship in root.descendants("Relationship") {
            let (Some(id), Some(target)) =
                (relationship.attr("Id"), relationship.attr("Target"))
            else {
                continue;
            };
            // An external relationship points outside the package and has no
            // part to resolve to.
            if relationship.attr("TargetMode") == Some("External") {
                continue;
            }
            if let Some(resolved) = package::resolve_relative(part, target) {
                map.insert(id.to_string(), resolved);
            }
        }
        map
    }

    fn import_page(
        &mut self,
        index: usize,
        page_node: &Node,
        relationships: &BTreeMap<String, String>,
    ) -> Result<Page, Error> {
        let title = page_node
            .attr("NameU")
            .or_else(|| page_node.attr("Name"))
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| format!("Page {}", index + 1));

        let id = self.mint_id("page");
        let mut page = Page::new(id, title);

        // The page sheet holds the paper size, which is also what the y axis
        // is flipped about.
        let page_cells = page_node
            .child("PageSheet")
            .map(sheet::cells_of)
            .unwrap_or_default();
        let width_in = number(&page_cells, "PageWidth").unwrap_or(8.5);
        let height_in = number(&page_cells, "PageHeight").unwrap_or(11.0);
        page.settings = Some(dc_model::PageSettings {
            size: Some(dc_model::PageSize::custom(
                width_in * POINTS_PER_INCH,
                height_in * POINTS_PER_INCH,
            )),
            ..Default::default()
        });

        let Some(part) = page_node
            .child("Rel")
            .and_then(|r| r.attr("id"))
            .and_then(|id| relationships.get(id))
        else {
            self.warnings
                .push(format!("page {} has no contents part", index + 1));
            return Ok(page);
        };
        let Some(text) = self.package.text(part) else {
            self.warnings
                .push(format!("page {} references a missing part", index + 1));
            return Ok(page);
        };
        let contents = xml::parse(&text)?;

        // Sheet id to the id the shape was given here, so connects can be
        // resolved after every shape exists.
        let mut by_sheet: HashMap<String, String> = HashMap::new();
        let mut connectors: Vec<(String, Sheet)> = Vec::new();

        // Only the page's own `Shapes` list seeds the walk. A group carries a
        // nested one, and collecting those here as well would import every
        // grouped shape a second time.
        let roots: Vec<&Node> = contents
            .children_named("PageContents")
            .chain(std::iter::once(&contents))
            .flat_map(|c| c.children_named("Shapes"))
            .flat_map(|s| s.children_named("Shape"))
            .collect();

        for shape_node in roots {
            self.import_shape(
                shape_node,
                height_in,
                &mut page,
                &mut by_sheet,
                &mut connectors,
            );
        }

        self.import_connects(&contents, &by_sheet, &connectors, height_in, &mut page);
        Ok(page)
    }

    /// Imports one shape and, when it is a group, everything inside it.
    fn import_shape(
        &mut self,
        node: &Node,
        page_height_in: f64,
        page: &mut Page,
        by_sheet: &mut HashMap<String, String>,
        connectors: &mut Vec<(String, Sheet)>,
    ) {
        let sheet_id = node.attr("ID").unwrap_or_default().to_string();
        let master_cells = node
            .attr("Master")
            .and_then(|m| self.masters.get(m))
            .map(|m| m.cells.clone())
            .unwrap_or_default();
        let master_name = node
            .attr("Master")
            .and_then(|m| self.masters.get(m))
            .map(|m| m.name.clone())
            .or_else(|| node.attr("NameU").map(str::to_string))
            .unwrap_or_default();

        let sheet = Sheet::read(node, &master_cells, &master_name, page_height_in);

        // A group is a frame around its members. Its members carry the
        // drawing, so the group itself is not emitted and its children are
        // lifted into the page.
        let is_group = node.attr("Type") == Some("Group");
        if is_group {
            for child in node
                .children_named("Shapes")
                .flat_map(|s| s.children_named("Shape"))
            {
                self.import_shape(child, page_height_in, page, by_sheet, connectors);
            }
            // A group with text of its own still deserves a label, so it is
            // kept as a text block when it has one.
            if sheet.text.trim().is_empty() {
                return;
            }
        }

        if masters::is_connector(&master_name) {
            connectors.push((sheet_id, sheet));
            return;
        }

        let shape_type = match masters::shape_type_for(&master_name) {
            Some(t) => t,
            None if is_group => ShapeType::TextBlock,
            None => {
                if !master_name.is_empty() {
                    let note = format!("the {master_name:?} stencil shape drew as a rectangle");
                    if !self.warnings.contains(&note) {
                        self.warnings.push(note);
                    }
                }
                ShapeType::Rectangle
            }
        };

        let id = self.mint_id("shape");
        let mut shape = Shape::new(&id, shape_type.clone(), sheet.bounds);
        if !sheet.text.trim().is_empty() {
            shape.text = Some(Text::new(escape_markup(&sheet.text)));
        }
        if shape_type.accepts_style() {
            shape.style = Some(sheet.style());
        }
        // The stencil the shape came from is worth keeping even when it did
        // not map onto a type, so a round trip does not lose what it was.
        if !master_name.is_empty() {
            shape.extra.insert(
                "vsdxMaster".to_string(),
                serde_json::Value::String(master_name.clone()),
            );
        }

        by_sheet.insert(sheet_id, id);
        page.add_shape(shape);
    }

    /// Turns the page's `Connects` into connectors between the imported shapes.
    fn import_connects(
        &mut self,
        contents: &Node,
        by_sheet: &HashMap<String, String>,
        connectors: &[(String, Sheet)],
        page_height_in: f64,
        page: &mut Page,
    ) {
        // Which shape each end of each connector is glued to.
        let mut ends: HashMap<String, (Option<String>, Option<String>)> = HashMap::new();
        for connect in contents.descendants("Connect") {
            let (Some(from), Some(to)) = (connect.attr("FromSheet"), connect.attr("ToSheet"))
            else {
                continue;
            };
            let Some(target) = by_sheet.get(to) else { continue };
            let entry = ends.entry(from.to_string()).or_default();

            // `FromCell` says which end of the connector is glued: BeginX is
            // the tail, EndX the head.
            match connect.attr("FromCell").unwrap_or("") {
                cell if cell.starts_with("Begin") => entry.0 = Some(target.clone()),
                cell if cell.starts_with("End") => entry.1 = Some(target.clone()),
                // Some writers glue by index instead of by cell name.
                _ => match connect.attr("FromPart").unwrap_or("") {
                    "9" => entry.0 = Some(target.clone()),
                    "12" => entry.1 = Some(target.clone()),
                    _ => {
                        if entry.0.is_none() {
                            entry.0 = Some(target.clone());
                        } else {
                            entry.1 = Some(target.clone());
                        }
                    }
                },
            }
        }

        for (sheet_id, sheet) in connectors {
            let (from, to) = ends.get(sheet_id).cloned().unwrap_or((None, None));

            // An end that is not glued keeps the coordinates the drawing gave
            // it, so a connector to empty space still lands where it did.
            let begin = sheet.begin_point(page_height_in);
            let end = sheet.end_point(page_height_in);

            let endpoint1 = match from {
                Some(id) => Endpoint::at_shape(id, EndpointStyle::None),
                None => Endpoint::at_point(begin, EndpointStyle::None),
            };
            let endpoint2 = match to {
                Some(id) => Endpoint::at_shape(id, EndpointStyle::Arrow),
                None => Endpoint::at_point(end, EndpointStyle::Arrow),
            };

            let id = self.mint_id("line");
            let mut line = Line::new(&id, endpoint1, endpoint2).with_line_type(sheet.line_type());
            line.stroke = Some(sheet.stroke());
            if !sheet.text.trim().is_empty() {
                line.text = Some(vec![dc_model::LineLabel::new(escape_markup(&sheet.text))]);
            }
            page.add_line(line);
        }
    }

    /// An id that no other imported item is using.
    fn mint_id(&mut self, prefix: &str) -> String {
        loop {
            let candidate = format!("{prefix}{}", self.next_id);
            self.next_id += 1;
            if self.used_ids.insert(candidate.clone()) {
                return candidate;
            }
        }
    }
}

/// Escapes text so that angle brackets a Visio label contains stay literal
/// rather than being read back as markup.
fn escape_markup(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\n' | '\r' => out.push_str("<br>"),
            other => out.push(other),
        }
    }
    // A paragraph marker at the end of a Visio label is a terminator, not a
    // blank line.
    out.trim_end_matches("<br>").to_string()
}

fn number(cells: &BTreeMap<String, String>, name: &str) -> Option<f64> {
    cells.get(name)?.trim().parse::<f64>().ok()
}

/// Reads a Visio boolean, which may be written as `1` or `true`.
fn is_true(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "True" | "TRUE")
}

#[cfg(test)]
mod tests;
