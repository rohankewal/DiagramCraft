//! Shapes.

use crate::geom::{BoundingBox, Point, RelPoint};
use crate::string_enum::string_enum;
use crate::style::{Stroke, Style, Text};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

string_enum! {
    /// A shape type from one of the published shape libraries.
    ///
    /// The types are declared library by library in the order the published
    /// reference lists them. Values from a library that has no documented type
    /// string land in `Other` and are preserved verbatim on save.
    ///
    /// AWS, Azure, and GCP shapes do not get their own type strings: the
    /// published cloud libraries express every icon as `namedShape` or
    /// `namedContainer` carrying a `className`, which is why those two appear
    /// here rather than several hundred service names.
    ShapeType {
        // Standard library.
        Rectangle => "rectangle",
        TextBlock => "text",
        Hotspot => "hotspot",
        Image => "image",
        StickyNote => "stickyNote",

        // Flowchart library.
        BraceNote => "braceNote",
        Connector => "connector",
        Database => "database",
        Data => "data",
        Decision => "decision",
        Delay => "delay",
        DirectAccessStorage => "directAccessStorage",
        Display => "display",
        Document => "document",
        InternalStorage => "internalStorage",
        ManualInput => "manualInput",
        ManualOperation => "manualOperation",
        Merge => "merge",
        MultipleDocuments => "multipleDocuments",
        Note => "note",
        OffPageLink => "offPageLink",
        Or => "or",
        PaperTape => "paperTape",
        PredefinedProcess => "predefinedProcess",
        Preparation => "preparation",
        Process => "process",
        StoredData => "storedData",
        SummingJunction => "summingJunction",
        Terminator => "terminator",

        // Shape library. `rectangle` is shared with the Standard library and is
        // declared once, above.
        Circle => "circle",
        Cloud => "cloud",
        Cross => "cross",
        Diamond => "diamond",
        DoubleArrow => "doubleArrow",
        FlexiblePolygon => "flexiblePolygon",
        Hexagon => "hexagon",
        IsoscelesTriangle => "isoscelesTriangle",
        Octagon => "octagon",
        Pentagon => "pentagon",
        PolyStar => "polyStar",
        RightTriangle => "rightTriangle",
        SingleArrow => "singleArrow",

        // Container library.
        BraceContainer => "braceContainer",
        BracketContainer => "bracketContainer",
        CircleContainer => "circleContainer",
        DiamondContainer => "diamondContainer",
        PillContainer => "pillContainer",
        RectangleContainer => "rectangleContainer",
        RoundedRectangleContainer => "roundedRectangleContainer",
        SwimLanes => "swimLanes",

        // Lucidspark library.
        FreehandDrawing => "freehandDrawing",
        SparkCalloutSquare => "sparkCalloutSquare",
        SparkContainer => "sparkContainer",
        SparkFrame => "sparkFrame",

        // BPMN 2.0 library.
        BpmnActivity => "bpmnActivity",
        BpmnBlackBoxPool => "bpmnBlackBoxPool",
        BpmnChoreography => "bpmnChoreography",
        BpmnConversation => "bpmnConversation",
        BpmnDataObject => "bpmnDataObject",
        BpmnDataStore => "bpmnDataStore",
        BpmnEvent => "bpmnEvent",
        BpmnGateway => "bpmnGateway",
        BpmnGroup => "bpmnGroup",
        BpmnPool => "bpmnPool",
        BpmnTextAnnotation => "bpmnTextAnnotation",

        // Table library.
        Table => "table",

        // AWS 2024, Azure 2021/2024, and GCP 2021 libraries.
        NamedShape => "namedShape",
        NamedContainer => "namedContainer",

        // ------------------------------------------------------------------
        // Beyond the published libraries.
        //
        // The notations below — UML, ERD, network, wireframe — are standards
        // or conventions in their own right, but the Standard Import format
        // publishes no types for them. Their strings are prefixed `dc` so
        // they cannot collide with a library Lucid may publish later. A file
        // using them round-trips here; another tool will carry them through
        // as unrecognised types rather than drawing them.

        // UML.
        UmlClass => "dcUmlClass",
        UmlInterface => "dcUmlInterface",
        UmlPackage => "dcUmlPackage",
        UmlActor => "dcUmlActor",
        UmlUseCase => "dcUmlUseCase",
        UmlLifeline => "dcUmlLifeline",
        UmlActivation => "dcUmlActivation",
        UmlComponent => "dcUmlComponent",
        UmlNode => "dcUmlNode",
        UmlState => "dcUmlState",
        UmlInitialState => "dcUmlInitialState",
        UmlFinalState => "dcUmlFinalState",
        UmlFork => "dcUmlFork",

        // Entity-relationship, which pairs with the crow's-foot endpoints
        // the published format already describes.
        ErdEntity => "dcErdEntity",
        ErdWeakEntity => "dcErdWeakEntity",
        ErdRelationship => "dcErdRelationship",
        ErdWeakRelationship => "dcErdWeakRelationship",
        ErdAttribute => "dcErdAttribute",
        ErdMultivaluedAttribute => "dcErdMultivaluedAttribute",
        ErdDerivedAttribute => "dcErdDerivedAttribute",

        // Network.
        NetServer => "dcNetServer",
        NetRouter => "dcNetRouter",
        NetSwitch => "dcNetSwitch",
        NetFirewall => "dcNetFirewall",
        NetWorkstation => "dcNetWorkstation",
        NetLaptop => "dcNetLaptop",
        NetMobile => "dcNetMobile",
        NetPrinter => "dcNetPrinter",
        NetRack => "dcNetRack",
        NetWireless => "dcNetWireless",

        // Wireframe and mockup.
        WireBrowser => "dcWireBrowser",
        WirePhone => "dcWirePhone",
        WireButton => "dcWireButton",
        WireTextField => "dcWireTextField",
        WireCheckbox => "dcWireCheckbox",
        WireRadio => "dcWireRadio",
        WireDropdown => "dcWireDropdown",
        WireImage => "dcWireImage",
        WireNavBar => "dcWireNavBar",
        WireSlider => "dcWireSlider",
        WireToggle => "dcWireToggle",

        // Charts and other diagram furniture.
        ChartVennTwo => "dcChartVennTwo",
        ChartVennThree => "dcChartVennThree",
        ChartMatrix => "dcChartMatrix",
        ChartSitemapPage => "dcChartSitemapPage",
        ChartTimeline => "dcChartTimeline",
        ChartMilestone => "dcChartMilestone",
    }
}

/// Which published library a shape type belongs to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ShapeLibrary {
    /// The library a palette opens on.
    #[default]
    Standard,
    Shape,
    Flowchart,
    Container,
    Bpmn,
    Table,
    Lucidspark,
    /// AWS, Azure, and GCP, which the format expresses as `namedShape` and
    /// `namedContainer` with a `className`.
    Cloud,
    /// Standard UML notation, which the published format does not cover.
    Uml,
    /// Entity-relationship entities and attributes.
    Erd,
    /// Network and infrastructure equipment.
    Network,
    /// Interface wireframing and mockups.
    Wireframe,
    /// Venn, matrix, sitemap and timeline furniture.
    Chart,
    /// Shapes someone starred. Its contents come from their own library
    /// rather than from a fixed list, so `types()` is empty here.
    Favorites,
    /// A type string no published library declares.
    Unsupported,
}

impl ShapeLibrary {
    /// Every library this build draws, in palette order.
    pub const ALL: &'static [ShapeLibrary] = &[
        ShapeLibrary::Standard,
        ShapeLibrary::Shape,
        ShapeLibrary::Flowchart,
        ShapeLibrary::Container,
        ShapeLibrary::Bpmn,
        ShapeLibrary::Table,
        ShapeLibrary::Lucidspark,
        ShapeLibrary::Cloud,
        ShapeLibrary::Uml,
        ShapeLibrary::Erd,
        ShapeLibrary::Network,
        ShapeLibrary::Wireframe,
        ShapeLibrary::Chart,
        ShapeLibrary::Favorites,
    ];

    pub fn display_name(self) -> &'static str {
        match self {
            ShapeLibrary::Standard => "Standard",
            ShapeLibrary::Shape => "Shapes",
            ShapeLibrary::Flowchart => "Flowchart",
            ShapeLibrary::Container => "Containers",
            ShapeLibrary::Bpmn => "BPMN 2.0",
            ShapeLibrary::Table => "Table",
            ShapeLibrary::Lucidspark => "Lucidspark",
            ShapeLibrary::Cloud => "AWS / Azure / GCP",
            ShapeLibrary::Uml => "UML",
            ShapeLibrary::Erd => "Entity Relationship",
            ShapeLibrary::Network => "Network",
            ShapeLibrary::Wireframe => "Wireframe",
            ShapeLibrary::Chart => "Charts",
            ShapeLibrary::Favorites => "My Shapes",
            ShapeLibrary::Unsupported => "Other",
        }
    }

    /// A short label that fits a palette tab.
    pub fn short_name(self) -> &'static str {
        match self {
            ShapeLibrary::Standard => "Std",
            ShapeLibrary::Shape => "Shapes",
            ShapeLibrary::Flowchart => "Flow",
            ShapeLibrary::Container => "Cont",
            ShapeLibrary::Bpmn => "BPMN",
            ShapeLibrary::Table => "Table",
            ShapeLibrary::Lucidspark => "Spark",
            ShapeLibrary::Cloud => "Cloud",
            ShapeLibrary::Uml => "UML",
            ShapeLibrary::Erd => "ERD",
            ShapeLibrary::Network => "Net",
            ShapeLibrary::Wireframe => "Wire",
            ShapeLibrary::Chart => "Chart",
            ShapeLibrary::Favorites => "Mine",
            ShapeLibrary::Unsupported => "Other",
        }
    }

    /// The types this library publishes, in declaration order.
    pub fn types(self) -> Vec<ShapeType> {
        ShapeType::KNOWN
            .iter()
            .map(|s| ShapeType::from(*s))
            .filter(|t| t.library() == self)
            .collect()
    }
}

/// Which cloud vendor a `className` names, read from the suffix the published
/// libraries give every class: `…AWS2024`, `…Azure2021`, `…Azure2024`, and the
/// `GCP2021…` prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloudVendor {
    Aws,
    Azure,
    Gcp,
}

impl CloudVendor {
    pub fn from_class_name(class_name: &str) -> Option<CloudVendor> {
        if class_name.starts_with("GCP") {
            Some(CloudVendor::Gcp)
        } else if class_name.ends_with("AWS2024") || class_name.contains("AWS20") {
            Some(CloudVendor::Aws)
        } else if class_name.contains("Azure20") {
            Some(CloudVendor::Azure)
        } else {
            None
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            CloudVendor::Aws => "AWS",
            CloudVendor::Azure => "Azure",
            CloudVendor::Gcp => "GCP",
        }
    }
}

impl ShapeType {
    pub fn library(&self) -> ShapeLibrary {
        use ShapeType::*;
        match self {
            Rectangle | TextBlock | Hotspot | Image | StickyNote => ShapeLibrary::Standard,

            Circle | Cloud | Cross | Diamond | DoubleArrow | FlexiblePolygon | Hexagon
            | IsoscelesTriangle | Octagon | Pentagon | PolyStar | RightTriangle | SingleArrow => {
                ShapeLibrary::Shape
            }

            BraceContainer | BracketContainer | CircleContainer | DiamondContainer
            | PillContainer | RectangleContainer | RoundedRectangleContainer | SwimLanes => {
                ShapeLibrary::Container
            }

            FreehandDrawing | SparkCalloutSquare | SparkContainer | SparkFrame => {
                ShapeLibrary::Lucidspark
            }

            BpmnActivity | BpmnBlackBoxPool | BpmnChoreography | BpmnConversation
            | BpmnDataObject | BpmnDataStore | BpmnEvent | BpmnGateway | BpmnGroup | BpmnPool
            | BpmnTextAnnotation => ShapeLibrary::Bpmn,

            Table => ShapeLibrary::Table,

            NamedShape | NamedContainer => ShapeLibrary::Cloud,

            UmlClass | UmlInterface | UmlPackage | UmlActor | UmlUseCase | UmlLifeline
            | UmlActivation | UmlComponent | UmlNode | UmlState | UmlInitialState
            | UmlFinalState | UmlFork => ShapeLibrary::Uml,

            ErdEntity | ErdWeakEntity | ErdRelationship | ErdWeakRelationship
            | ErdAttribute | ErdMultivaluedAttribute | ErdDerivedAttribute => ShapeLibrary::Erd,

            NetServer | NetRouter | NetSwitch | NetFirewall | NetWorkstation | NetLaptop
            | NetMobile | NetPrinter | NetRack | NetWireless => ShapeLibrary::Network,

            WireBrowser | WirePhone | WireButton | WireTextField | WireCheckbox | WireRadio
            | WireDropdown | WireImage | WireNavBar | WireSlider | WireToggle => {
                ShapeLibrary::Wireframe
            }

            ChartVennTwo | ChartVennThree | ChartMatrix | ChartSitemapPage | ChartTimeline
            | ChartMilestone => ShapeLibrary::Chart,

            Other(_) => ShapeLibrary::Unsupported,

            _ => ShapeLibrary::Flowchart,
        }
    }

    /// Whether the format permits a `text` property on this type.
    ///
    /// Three published types reject text: the hotspot block, `or`, and
    /// `summingJunction`.
    pub fn accepts_text(&self) -> bool {
        !matches!(
            self,
            ShapeType::Hotspot | ShapeType::Or | ShapeType::SummingJunction
        )
    }

    /// Whether the format permits a `style` property on this type.
    ///
    /// The text block is the one published type that rejects it.
    pub fn accepts_style(&self) -> bool {
        !matches!(self, ShapeType::TextBlock)
    }

    /// Whether other shapes dropped inside this one become its children.
    ///
    /// Containers are the published types that carry `magnetize`, plus the
    /// BPMN pool and group, which behave the same way.
    pub fn is_container(&self) -> bool {
        use ShapeType::*;
        matches!(
            self,
            BraceContainer
                | BracketContainer
                | CircleContainer
                | DiamondContainer
                | PillContainer
                | RectangleContainer
                | RoundedRectangleContainer
                | SwimLanes
                | SparkContainer
                | SparkFrame
                | BpmnPool
                | BpmnBlackBoxPool
                | BpmnGroup
                | NamedContainer
        )
    }

    /// A human label for the shape palette.
    pub fn display_name(&self) -> &str {
        use ShapeType::*;
        match self {
            Rectangle => "Rectangle",
            TextBlock => "Text",
            Hotspot => "Hotspot",
            Image => "Image",
            StickyNote => "Sticky Note",

            BraceNote => "Brace Note",
            Connector => "Connector",
            Database => "Database",
            Data => "Data",
            Decision => "Decision",
            Delay => "Delay",
            DirectAccessStorage => "Direct Access Storage",
            Display => "Display",
            Document => "Document",
            InternalStorage => "Internal Storage",
            ManualInput => "Manual Input",
            ManualOperation => "Manual Operation",
            Merge => "Merge",
            MultipleDocuments => "Multiple Documents",
            Note => "Note",
            OffPageLink => "Off-page Link",
            Or => "Or",
            PaperTape => "Paper Tape",
            PredefinedProcess => "Predefined Process",
            Preparation => "Preparation",
            Process => "Process",
            StoredData => "Stored Data",
            SummingJunction => "Summing Junction",
            Terminator => "Terminator",

            Circle => "Circle",
            Cloud => "Cloud",
            Cross => "Cross",
            Diamond => "Diamond",
            DoubleArrow => "Double Arrow",
            FlexiblePolygon => "Polygon",
            Hexagon => "Hexagon",
            IsoscelesTriangle => "Triangle",
            Octagon => "Octagon",
            Pentagon => "Pentagon",
            PolyStar => "Star",
            RightTriangle => "Right Triangle",
            SingleArrow => "Arrow",

            BraceContainer => "Brace",
            BracketContainer => "Bracket",
            CircleContainer => "Circle",
            DiamondContainer => "Diamond",
            PillContainer => "Pill",
            RectangleContainer => "Rectangle",
            RoundedRectangleContainer => "Rounded",
            SwimLanes => "Swim Lanes",

            FreehandDrawing => "Freehand",
            SparkCalloutSquare => "Callout",
            SparkContainer => "Container",
            SparkFrame => "Frame",

            BpmnActivity => "Activity",
            BpmnBlackBoxPool => "Black Box Pool",
            BpmnChoreography => "Choreography",
            BpmnConversation => "Conversation",
            BpmnDataObject => "Data Object",
            BpmnDataStore => "Data Store",
            BpmnEvent => "Event",
            BpmnGateway => "Gateway",
            BpmnGroup => "Group",
            BpmnPool => "Pool",
            BpmnTextAnnotation => "Annotation",

            Table => "Table",

            NamedShape => "Cloud Icon",
            NamedContainer => "Cloud Container",

            UmlClass => "Class",
            UmlInterface => "Interface",
            UmlPackage => "Package",
            UmlActor => "Actor",
            UmlUseCase => "Use Case",
            UmlLifeline => "Lifeline",
            UmlActivation => "Activation",
            UmlComponent => "Component",
            UmlNode => "Node",
            UmlState => "State",
            UmlInitialState => "Initial State",
            UmlFinalState => "Final State",
            UmlFork => "Fork / Join",

            ErdEntity => "Entity",
            ErdWeakEntity => "Weak Entity",
            ErdRelationship => "Relationship",
            ErdWeakRelationship => "Weak Relationship",
            ErdAttribute => "Attribute",
            ErdMultivaluedAttribute => "Multivalued",
            ErdDerivedAttribute => "Derived",

            NetServer => "Server",
            NetRouter => "Router",
            NetSwitch => "Switch",
            NetFirewall => "Firewall",
            NetWorkstation => "Workstation",
            NetLaptop => "Laptop",
            NetMobile => "Mobile",
            NetPrinter => "Printer",
            NetRack => "Rack",
            NetWireless => "Access Point",

            WireBrowser => "Browser",
            WirePhone => "Phone",
            WireButton => "Button",
            WireTextField => "Text Field",
            WireCheckbox => "Checkbox",
            WireRadio => "Radio",
            WireDropdown => "Dropdown",
            WireImage => "Image Block",
            WireNavBar => "Nav Bar",
            WireSlider => "Slider",
            WireToggle => "Toggle",

            ChartVennTwo => "Venn (2)",
            ChartVennThree => "Venn (3)",
            ChartMatrix => "2×2 Matrix",
            ChartSitemapPage => "Page",
            ChartTimeline => "Timeline",
            ChartMilestone => "Milestone",

            Other(s) => s,
        }
    }

    /// The size used when this shape is dragged out of the palette, in points.
    /// Chosen to match conventional symbol proportions.
    pub fn default_size(&self) -> (f64, f64) {
        use ShapeType::*;
        match self {
            TextBlock => (120.0, 30.0),
            StickyNote => (120.0, 120.0),
            Hotspot => (120.0, 60.0),
            Decision | Merge | Or | SummingJunction => (100.0, 100.0),
            Terminator => (120.0, 50.0),
            Connector => (40.0, 40.0),
            BraceNote => (40.0, 120.0),
            Note => (120.0, 80.0),

            Circle | Diamond | Cross | Hexagon | IsoscelesTriangle | Octagon | Pentagon
            | PolyStar | RightTriangle | FlexiblePolygon => (100.0, 100.0),
            SingleArrow | DoubleArrow => (140.0, 60.0),
            Cloud => (140.0, 90.0),

            BraceContainer | BracketContainer => (60.0, 200.0),
            CircleContainer | DiamondContainer => (240.0, 240.0),
            PillContainer | RectangleContainer | RoundedRectangleContainer => (280.0, 200.0),
            SwimLanes => (480.0, 300.0),

            FreehandDrawing => (160.0, 100.0),
            SparkCalloutSquare => (160.0, 100.0),
            SparkContainer | SparkFrame => (320.0, 240.0),

            BpmnActivity => (120.0, 80.0),
            BpmnEvent | BpmnGateway => (56.0, 56.0),
            BpmnDataObject => (70.0, 90.0),
            BpmnDataStore => (90.0, 80.0),
            BpmnConversation => (70.0, 60.0),
            BpmnChoreography => (140.0, 100.0),
            BpmnPool | BpmnBlackBoxPool => (480.0, 180.0),
            BpmnGroup => (280.0, 200.0),
            BpmnTextAnnotation => (160.0, 70.0),

            Table => (300.0, 160.0),

            NamedShape => (80.0, 80.0),
            NamedContainer => (280.0, 200.0),

            // A class box needs room for three compartments; a lifeline is
            // tall because its dashed tail is most of the shape.
            UmlClass => (160.0, 120.0),
            UmlInterface => (160.0, 120.0),
            UmlPackage => (180.0, 120.0),
            UmlActor => (40.0, 80.0),
            UmlUseCase => (140.0, 70.0),
            UmlLifeline => (120.0, 220.0),
            UmlActivation => (16.0, 120.0),
            UmlComponent => (160.0, 80.0),
            UmlNode => (150.0, 100.0),
            UmlState => (140.0, 70.0),
            UmlInitialState | UmlFinalState => (28.0, 28.0),
            UmlFork => (160.0, 10.0),

            ErdEntity | ErdWeakEntity => (160.0, 110.0),
            ErdRelationship | ErdWeakRelationship => (130.0, 80.0),
            ErdAttribute | ErdMultivaluedAttribute | ErdDerivedAttribute => (120.0, 55.0),

            NetServer | NetRack => (80.0, 110.0),
            NetRouter | NetSwitch | NetWireless => (90.0, 60.0),
            NetFirewall => (100.0, 70.0),
            NetWorkstation | NetPrinter => (100.0, 85.0),
            NetLaptop => (110.0, 75.0),
            NetMobile => (50.0, 90.0),

            WireBrowser => (320.0, 220.0),
            WirePhone => (160.0, 300.0),
            WireButton => (110.0, 36.0),
            WireTextField | WireDropdown => (180.0, 34.0),
            WireCheckbox | WireRadio => (18.0, 18.0),
            WireImage => (180.0, 120.0),
            WireNavBar => (320.0, 40.0),
            WireSlider => (180.0, 20.0),
            WireToggle => (46.0, 24.0),

            ChartVennTwo => (220.0, 140.0),
            ChartVennThree => (200.0, 180.0),
            ChartMatrix => (220.0, 220.0),
            ChartSitemapPage => (110.0, 70.0),
            ChartTimeline => (320.0, 40.0),
            ChartMilestone => (36.0, 36.0),

            _ => (120.0, 60.0),
        }
    }
}

/// The `shape` property on a poly-star block.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StarShape {
    /// Published range: 3 to 25.
    pub num_points: u32,
    /// Published range: 0 to 1, as a fraction of the outer radius.
    pub inner_radius: f64,
}

impl StarShape {
    pub fn resolved_points(&self) -> u32 {
        self.num_points.clamp(3, 25)
    }

    pub fn resolved_inner_radius(&self) -> f64 {
        self.inner_radius.clamp(0.0, 1.0)
    }
}

impl Default for StarShape {
    fn default() -> Self {
        StarShape { num_points: 5, inner_radius: 0.4 }
    }
}

/// An image reference on an image block.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageRef {
    /// A file in the package's `images` directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    /// An external URL, used when `ref` is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// An action wired to a shape: a hyperlink, a jump to another page or
/// document, or a layer-visibility toggle.
///
/// Source: `reference-si` on developer.lucid.co, which documents these six
/// action types and the fields each one carries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    /// Opens a URL.
    #[serde(rename_all = "camelCase")]
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_window: Option<bool>,
    },
    /// Jumps to a page in this document.
    #[serde(rename_all = "camelCase")]
    GotoPage { page_id: String },
    /// Jumps to a page in another document.
    #[serde(rename_all = "camelCase")]
    GotoDocument {
        document_id: String,
        page_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        new_window: Option<bool>,
    },
    /// Hides the named layers.
    #[serde(rename_all = "camelCase")]
    HideLayer { layers: Vec<String> },
    /// Shows the named layers.
    #[serde(rename_all = "camelCase")]
    ShowLayer { layers: Vec<String> },
    /// Flips the named layers between hidden and shown.
    #[serde(rename_all = "camelCase")]
    ToggleLayer { layers: Vec<String> },
}

/// A key-value pair attached to a shape, page, or layer.
///
/// Source: `reference-si`, the "Custom Data" type.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CustomDataEntry {
    pub key: String,
    pub value: String,
}

/// A shape on a page.
///
/// Fields published as common to most shapes are typed. The handful of
/// type-specific published fields are typed too. Anything else the document
/// carries is captured in `extra` so that saving does not discard it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    pub id: String,
    #[serde(rename = "type")]
    pub shape_type: ShapeType,
    pub bounding_box: BoundingBox,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub style: Option<Style>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Text>,
    /// Percentage, 0 to 100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub z_index: Option<i64>,
    /// Navigation and layer-visibility actions wired to the shape, for
    /// example a hyperlink.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
    /// Arbitrary key-value pairs attached to the shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_data: Option<Vec<CustomDataEntry>>,
    /// Whether the shape is pinned against editing.
    ///
    /// An extension: the published specification gives layers a lock but not
    /// shapes. It is written only once something sets it, and a reader that
    /// does not know the key carries it through untouched the same way it
    /// carries any other field it does not interpret — so adding it costs a
    /// round trip nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,

    /// Required on the image block, which carries its stroke outside `style`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Stroke>,
    /// Required on the image block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImageRef>,

    /// Brace orientation on `braceNote`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_facing: Option<bool>,
    /// Brace thickness on `braceNote`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brace_width: Option<f64>,
    /// Side bar width on `predefinedProcess`, from 0 to 0.33.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_width: Option<f64>,

    // ------------------------------------------------- Shape library

    /// Arm thickness on `cross`, each component from 0 to 0.5.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub indent: Option<RelPoint>,
    /// Outline of a `flexiblePolygon`, 3 to 100 relative positions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertices: Option<Vec<RelPoint>>,
    /// Point count and waist of a `polyStar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<StarShape>,
    /// Which way a `singleArrow` points: `right`, `left`, `up`, or `down`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orientation: Option<String>,

    // --------------------------------- Container and Lucidspark libraries

    /// Whether contained shapes move with the container. Defaults to true.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magnetize: Option<bool>,
    /// Whether contained items are auto-arranged into a grid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assisted_layout: Option<bool>,
    /// Lane orientation on `swimLanes` and `bpmnPool`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical: Option<bool>,
    /// Stroke path of a `freehandDrawing`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<Point>>,
    /// Origin the `points` of a `freehandDrawing` are measured from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<Point>,

    // -------------------------------------------------- BPMN 2.0 library

    /// `task`, `transaction`, `eventSubProcess`, or `callActivity`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_type: Option<String>,
    /// The icon in the corner of a BPMN task.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_type: Option<String>,
    /// `start`, `intermediate`, or `end`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_group: Option<String>,
    /// The trigger drawn inside a BPMN event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    /// Whether a BPMN event interrupts its enclosing activity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub non_interrupting: Option<bool>,
    /// Whether a BPMN event throws rather than catches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub throwing: Option<bool>,
    /// The marker inside a BPMN gateway diamond.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_type: Option<String>,
    /// `none`, `collection`, `input`, or `output` on a BPMN data object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    /// `task`, `subchoreography`, or `call`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choreography_type: Option<String>,
    /// Whether a BPMN conversation is a call conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_call: Option<bool>,
    /// Whether a BPMN conversation is a sub-conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_sub_conversation: Option<bool>,

    // ----------------------------------------------------- Table library

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vertical_border: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub horizontal_border: Option<bool>,

    // --------------------------------------- AWS, Azure, and GCP libraries

    /// Which icon a `namedShape` or `namedContainer` draws, for example
    /// `ArchAmazonEC2AWS2024`, `VirtualMachineAzure2024`, or
    /// `GCP2021CloudStorageIcon`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub class_name: Option<String>,

    /// Properties not modeled above, preserved for round-tripping.
    #[serde(flatten, default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Shape {
    pub fn new(id: impl Into<String>, shape_type: ShapeType, bounding_box: BoundingBox) -> Self {
        Shape {
            id: id.into(),
            shape_type,
            bounding_box,
            style: None,
            text: None,
            opacity: None,
            note: None,
            z_index: None,
            actions: None,
            custom_data: None,
            locked: None,
            stroke: None,
            image: None,
            right_facing: None,
            brace_width: None,
            side_width: None,
            indent: None,
            vertices: None,
            shape: None,
            orientation: None,
            magnetize: None,
            assisted_layout: None,
            vertical: None,
            points: None,
            offset: None,
            activity_type: None,
            task_type: None,
            event_group: None,
            event_type: None,
            non_interrupting: None,
            throwing: None,
            gateway_type: None,
            data_type: None,
            choreography_type: None,
            is_call: None,
            is_sub_conversation: None,
            row_count: None,
            col_count: None,
            vertical_border: None,
            horizontal_border: None,
            class_name: None,
            extra: Map::new(),
        }
    }

    /// Sets the icon class on a `namedShape` or `namedContainer`.
    pub fn with_class_name(mut self, class_name: impl Into<String>) -> Self {
        self.class_name = Some(class_name.into());
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(Text::new(text));
        self
    }

    pub fn with_style(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Opacity as a 0..=1 multiplier. The document stores a percentage.
    pub fn opacity_factor(&self) -> f64 {
        self.opacity.map_or(1.0, |o| (o / 100.0).clamp(0.0, 1.0))
    }

    pub fn resolved_z_index(&self) -> i64 {
        self.z_index.unwrap_or(0)
    }

    pub fn resolved_style(&self) -> Style {
        self.style.clone().unwrap_or_default()
    }

    /// The stroke to draw, preferring the type-specific top-level stroke that
    /// the image block uses over the one inside `style`.
    pub fn effective_stroke(&self) -> Stroke {
        self.stroke.unwrap_or_else(|| self.resolved_style().resolved_stroke())
    }

    pub fn text_str(&self) -> &str {
        self.text.as_ref().map_or("", |t| t.as_str())
    }

    /// The side bar fraction for `predefinedProcess`, clamped to the published
    /// 0 to 0.33 range.
    pub fn resolved_side_width(&self) -> f64 {
        self.side_width.unwrap_or(0.1).clamp(0.0, 0.33)
    }

    /// The arm thickness of a `cross`, clamped to the published 0 to 0.5 range.
    pub fn resolved_indent(&self) -> RelPoint {
        let i = self.indent.unwrap_or(RelPoint::new(0.3, 0.3));
        RelPoint::new(i.x.clamp(0.0, 0.5), i.y.clamp(0.0, 0.5))
    }

    /// The outline of a `flexiblePolygon`, falling back to a pentagon when the
    /// property is absent or has too few vertices to enclose an area.
    pub fn resolved_vertices(&self) -> Vec<RelPoint> {
        match &self.vertices {
            Some(v) if v.len() >= 3 => v.iter().take(100).copied().collect(),
            _ => regular_polygon_vertices(5),
        }
    }

    pub fn resolved_star(&self) -> StarShape {
        self.shape.unwrap_or_default()
    }

    /// Whether contained shapes move with this container. Defaults to true.
    /// Whether the shape refuses to be moved, resized, or deleted.
    pub fn is_locked(&self) -> bool {
        self.locked.unwrap_or(false)
    }

    pub fn is_magnetized(&self) -> bool {
        self.magnetize.unwrap_or(true)
    }

    /// Whether a lane container runs vertically. Defaults to horizontal lanes.
    pub fn is_vertical(&self) -> bool {
        self.vertical.unwrap_or(false)
    }

    /// The vendor of a `namedShape` or `namedContainer`, from its class name.
    pub fn cloud_vendor(&self) -> Option<CloudVendor> {
        CloudVendor::from_class_name(self.class_name.as_deref()?)
    }

    /// How many rows and columns a `table` has, with the format's minimum of
    /// one each so a malformed table still draws.
    pub fn resolved_table_size(&self) -> (u32, u32) {
        (
            self.row_count.unwrap_or(2).clamp(1, 200),
            self.col_count.unwrap_or(2).clamp(1, 200),
        )
    }
}

/// The vertices of a regular polygon inscribed in the unit box, starting at the
/// top and going clockwise.
pub fn regular_polygon_vertices(sides: u32) -> Vec<RelPoint> {
    let n = sides.max(3);
    (0..n)
        .map(|i| {
            let angle = -std::f64::consts::FRAC_PI_2
                + i as f64 * std::f64::consts::TAU / n as f64;
            RelPoint::new(0.5 + 0.5 * angle.cos(), 0.5 + 0.5 * angle.sin())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    fn count_in(library: ShapeLibrary) -> usize {
        ShapeType::KNOWN
            .iter()
            .filter(|s| ShapeType::from(**s).library() == library)
            .count()
    }

    #[test]
    fn known_counts_match_published_libraries() {
        // Counts taken from the published library references. The Shape library
        // publishes 14 types, one of which — `rectangle` — it shares with the
        // Standard library and which is therefore counted there.
        assert_eq!(count_in(ShapeLibrary::Standard), 5, "Standard publishes 5");
        assert_eq!(count_in(ShapeLibrary::Flowchart), 24, "Flowchart publishes 24");
        assert_eq!(count_in(ShapeLibrary::Shape), 13, "Shape publishes 14, sharing rectangle");
        assert_eq!(count_in(ShapeLibrary::Container), 8, "Container publishes 8");
        assert_eq!(count_in(ShapeLibrary::Lucidspark), 4, "Lucidspark publishes 4");
        assert_eq!(count_in(ShapeLibrary::Bpmn), 11, "BPMN 2.0 publishes 11");
        assert_eq!(count_in(ShapeLibrary::Table), 1, "Table publishes 1");
        assert_eq!(count_in(ShapeLibrary::Cloud), 2, "namedShape and namedContainer");
    }

    #[test]
    fn every_known_type_belongs_to_a_real_library() {
        for name in ShapeType::KNOWN {
            let t = ShapeType::from(*name);
            assert_ne!(
                t.library(),
                ShapeLibrary::Unsupported,
                "{name} is published but has no library"
            );
            assert!(ShapeLibrary::ALL.contains(&t.library()), "{name}");
        }
    }

    #[test]
    fn library_types_partition_the_known_set() {
        let listed: usize = ShapeLibrary::ALL.iter().map(|l| l.types().len()).sum();
        assert_eq!(listed, ShapeType::KNOWN.len());
    }

    #[test]
    fn unknown_type_is_preserved_not_dropped() {
        let json = r#"{"id":"s1","type":"someFutureBlock","boundingBox":{"x":0,"y":0,"w":1,"h":1}}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.shape_type, ShapeType::Other("someFutureBlock".into()));
        assert!(!s.shape_type.is_known());
        assert_eq!(s.shape_type.library(), ShapeLibrary::Unsupported);

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["type"], "someFutureBlock");
    }

    #[test]
    fn published_bpmn_example_parses() {
        let json = r#"{"id":"s1","type":"bpmnEvent",
            "boundingBox":{"x":0,"y":0,"w":40,"h":40},
            "eventGroup":"start","eventType":"message","nonInterrupting":false}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.shape_type, ShapeType::BpmnEvent);
        assert_eq!(s.event_group.as_deref(), Some("start"));
        assert_eq!(s.event_type.as_deref(), Some("message"));

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["eventGroup"], "start");
        assert_eq!(back["nonInterrupting"], false);
    }

    #[test]
    fn published_cloud_example_parses() {
        // Verbatim shape from the AWS 2024 library reference.
        let json = r#"{"id":"shape1","type":"namedShape",
            "className":"ArchAmazonSimpleStorageServiceAWS2024",
            "boundingBox":{"x":0,"y":0,"w":80,"h":80},"text":"S3 Bucket"}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.shape_type, ShapeType::NamedShape);
        assert_eq!(s.cloud_vendor(), Some(CloudVendor::Aws));

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["className"], "ArchAmazonSimpleStorageServiceAWS2024");
    }

    #[test]
    fn cloud_vendor_is_read_from_the_published_class_naming() {
        assert_eq!(
            CloudVendor::from_class_name("GCP2021CloudStorageIcon"),
            Some(CloudVendor::Gcp)
        );
        assert_eq!(
            CloudVendor::from_class_name("VirtualMachineAzure2024"),
            Some(CloudVendor::Azure)
        );
        assert_eq!(
            CloudVendor::from_class_name("ArchAWSLambdaAWS2024"),
            Some(CloudVendor::Aws)
        );
        assert_eq!(CloudVendor::from_class_name("NotACloudShape"), None);
    }

    #[test]
    fn container_types_are_flagged_as_containers() {
        assert!(ShapeType::RectangleContainer.is_container());
        assert!(ShapeType::SwimLanes.is_container());
        assert!(ShapeType::BpmnPool.is_container());
        assert!(!ShapeType::Rectangle.is_container());
        assert!(!ShapeType::BpmnEvent.is_container());
    }

    #[test]
    fn published_shape_library_properties_parse() {
        let json = r#"{"id":"s1","type":"polyStar",
            "boundingBox":{"x":0,"y":0,"w":100,"h":100},
            "shape":{"numPoints":7,"innerRadius":0.55}}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.resolved_star().resolved_points(), 7);
        assert!((s.resolved_star().resolved_inner_radius() - 0.55).abs() < 1e-9);

        let cross: Shape = serde_json::from_str(
            r#"{"id":"s2","type":"cross","boundingBox":{"x":0,"y":0,"w":10,"h":10},
                "indent":{"x":0.9,"y":0.2}}"#,
        )
        .unwrap();
        assert_eq!(cross.resolved_indent().x, 0.5, "indent clamps to the published range");
    }

    #[test]
    fn star_points_clamp_to_the_published_range() {
        let s = StarShape { num_points: 99, inner_radius: 5.0 };
        assert_eq!(s.resolved_points(), 25);
        assert_eq!(s.resolved_inner_radius(), 1.0);
    }

    #[test]
    fn flexible_polygon_falls_back_when_degenerate() {
        let mut s = Shape::new("s", ShapeType::FlexiblePolygon, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(s.resolved_vertices().len(), 5, "a pentagon stands in");
        s.vertices = Some(vec![RelPoint::new(0.0, 0.0), RelPoint::new(1.0, 1.0)]);
        assert_eq!(s.resolved_vertices().len(), 5, "two points enclose nothing");

        s.vertices = Some(vec![
            RelPoint::new(0.5, 0.0),
            RelPoint::new(1.0, 1.0),
            RelPoint::new(0.0, 1.0),
        ]);
        assert_eq!(s.resolved_vertices().len(), 3);
    }

    #[test]
    fn regular_polygons_are_inscribed_in_the_unit_box() {
        for sides in [3u32, 5, 8, 12] {
            let v = regular_polygon_vertices(sides);
            assert_eq!(v.len() as u32, sides);
            for p in &v {
                assert!((-0.001..=1.001).contains(&p.x), "{sides}: {p:?}");
                assert!((-0.001..=1.001).contains(&p.y), "{sides}: {p:?}");
            }
            assert!(v[0].y < 0.001, "the first vertex is at the top");
        }
    }

    #[test]
    fn container_and_table_properties_round_trip() {
        let json = r#"{"id":"t1","type":"table",
            "boundingBox":{"x":0,"y":0,"w":300,"h":160},
            "rowCount":3,"colCount":4,"verticalBorder":true,
            "cells":[{"row":0,"col":0,"text":"A"}]}"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.resolved_table_size(), (3, 4));
        assert!(s.extra.contains_key("cells"), "untyped cells are kept");

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["rowCount"], 3);
        assert_eq!(back["cells"][0]["text"], "A");
    }

    #[test]
    fn lucidspark_freehand_drawing_keeps_its_stroke() {
        let json = r##"{"id":"f1","type":"freehandDrawing",
            "boundingBox":{"x":0,"y":0,"w":100,"h":100},
            "style":{"width":4,"color":"#ff0000"},
            "points":[{"x":0,"y":0},{"x":10,"y":20}]}"##;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert_eq!(s.points.as_ref().unwrap().len(), 2);

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["points"][1]["y"], 20.0);
        assert_eq!(
            back["style"]["width"], 4,
            "the freehand stroke style must survive a round trip"
        );
    }

    #[test]
    fn unmodeled_properties_survive_round_trip() {
        let json = r#"{
            "id":"s1","type":"rectangle",
            "boundingBox":{"x":0,"y":0,"w":10,"h":10},
            "someFutureField":{"nested":[1,2,3]}
        }"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert!(s.extra.contains_key("someFutureField"));

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["someFutureField"]["nested"][2], 3);
    }

    #[test]
    fn a_url_action_is_typed_rather_than_opaque() {
        let json = r#"{
            "id":"s1","type":"rectangle",
            "boundingBox":{"x":0,"y":0,"w":10,"h":10},
            "actions":[{"type":"url","url":"https://example.com","newWindow":true}]
        }"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert!(!s.extra.contains_key("actions"), "it should be a typed field now");
        assert_eq!(
            s.actions,
            Some(vec![Action::Url { url: "https://example.com".into(), new_window: Some(true) }])
        );

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["actions"][0]["type"], "url");
        assert_eq!(back["actions"][0]["url"], "https://example.com");
        assert_eq!(back["actions"][0]["newWindow"], true);
    }

    #[test]
    fn every_documented_action_type_round_trips() {
        for json in [
            r#"{"type":"url","url":"https://example.com"}"#,
            r#"{"type":"gotoPage","pageId":"p1"}"#,
            r#"{"type":"gotoDocument","documentId":"d1","pageId":"p1"}"#,
            r#"{"type":"hideLayer","layers":["l1","l2"]}"#,
            r#"{"type":"showLayer","layers":["l1"]}"#,
            r#"{"type":"toggleLayer","layers":["l1"]}"#,
        ] {
            let action: Action = serde_json::from_str(json).unwrap();
            let back = serde_json::to_value(&action).unwrap();
            let original: serde_json::Value = serde_json::from_str(json).unwrap();
            assert_eq!(back, original, "{json} did not round-trip");
        }
    }

    #[test]
    fn custom_data_is_typed_rather_than_opaque() {
        let json = r#"{
            "id":"s1","type":"rectangle",
            "boundingBox":{"x":0,"y":0,"w":10,"h":10},
            "customData":[{"key":"owner","value":"ops"}]
        }"#;
        let s: Shape = serde_json::from_str(json).unwrap();
        assert!(!s.extra.contains_key("customData"));
        assert_eq!(
            s.custom_data,
            Some(vec![CustomDataEntry { key: "owner".into(), value: "ops".into() }])
        );

        let back = serde_json::to_value(&s).unwrap();
        assert_eq!(back["customData"][0]["key"], "owner");
        assert_eq!(back["customData"][0]["value"], "ops");
    }

    #[test]
    fn published_type_constraints_are_encoded() {
        assert!(!ShapeType::Hotspot.accepts_text());
        assert!(!ShapeType::Or.accepts_text());
        assert!(!ShapeType::SummingJunction.accepts_text());
        assert!(ShapeType::Rectangle.accepts_text());

        assert!(!ShapeType::TextBlock.accepts_style());
        assert!(ShapeType::Rectangle.accepts_style());
    }

    #[test]
    fn opacity_converts_percentage_to_factor() {
        let mut s = Shape::new("a", ShapeType::Rectangle, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(s.opacity_factor(), 1.0);
        s.opacity = Some(50.0);
        assert_eq!(s.opacity_factor(), 0.5);
        s.opacity = Some(250.0);
        assert_eq!(s.opacity_factor(), 1.0, "out-of-range values clamp");
    }

    #[test]
    fn image_block_stroke_wins_over_style_stroke() {
        let mut s = Shape::new("i", ShapeType::Image, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        s.style = Some(Style { stroke: Some(Stroke::new(Color::BLACK, 9.0)), ..Default::default() });
        s.stroke = Some(Stroke::new(Color::WHITE, 2.0));
        assert_eq!(s.effective_stroke().resolved_width(), 2.0);
    }

    #[test]
    fn side_width_clamps_to_published_range() {
        let mut s = Shape::new("p", ShapeType::PredefinedProcess, BoundingBox::new(0.0, 0.0, 1.0, 1.0));
        s.side_width = Some(0.9);
        assert_eq!(s.resolved_side_width(), 0.33);
    }
}
