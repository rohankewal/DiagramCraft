# Format provenance

This document records where DiagramCraft's file-format knowledge came from and
how it was used. It exists so that anyone reviewing the project — a
contributor, a downstream distributor, or a lawyer — can verify the
methodology without taking anyone's word for it.

## Summary

DiagramCraft reads and writes the **Lucid Standard Import** format, which Lucid
publishes as developer documentation. No part of this project was derived from
decompiling, disassembling, or instrumenting another program. No binary was
examined. No minified source was recovered. No proprietary asset was copied.

The format is documented publicly because Lucid wants third parties to write
importers. We are a third party writing an importer.

## Sources

Every structural fact in `dc-model` and `dc-lucid` traces to one of these
pages, all publicly readable without an account:

| Topic | Source |
|---|---|
| Package layout, `document.json`, 2 MB limit | <https://developer.lucid.co/docs/overview-si> |
| Shared types: bounding box, position, fill, stroke, style, text, ID rules, the `Action` type (hyperlinks and navigation), Custom Data | <https://developer.lucid.co/docs/reference-si> |
| Page object and page settings | <https://developer.lucid.co/docs/pages-si> |
| Shape common properties, and the list of libraries | <https://developer.lucid.co/docs/shapes-si> |
| Standard library (5 shape types) | <https://developer.lucid.co/docs/standard-library-si> |
| Shape library (14 shape types) | <https://developer.lucid.co/docs/shape-library-si> |
| Flowchart library (24 shape types) | <https://developer.lucid.co/docs/flowchart-library-si> |
| Container library (8 shape types) | <https://developer.lucid.co/docs/container-library-si> |
| BPMN 2.0 library (11 shape types) | <https://developer.lucid.co/docs/bpmn-20-library-si> |
| Table library (1 shape type) | <https://developer.lucid.co/docs/table-library-si> |
| Lucidspark library (4 shape types) | <https://developer.lucid.co/docs/lucidspark-library-si> |
| AWS 2024 library (`namedShape` / `namedContainer` + `className`) | <https://developer.lucid.co/docs/aws-2024-library> |
| Azure 2024 library | <https://developer.lucid.co/docs/azure-2024-library> |
| GCP 2021 library | <https://developer.lucid.co/docs/gcp-2021-library> |
| Lines, endpoints, endpoint styles, labels | <https://developer.lucid.co/docs/lines-si> |
| Groups | <https://developer.lucid.co/docs/groups-si> |
| Layers | <https://developer.lucid.co/docs/layers-si> |

The Shape library shares its `rectangle` type with the Standard library, so the
68 types this build implements are 5 + 13 + 24 + 8 + 11 + 1 + 4 + 2 rather than
the sum of the published counts.

### The cloud libraries

AWS, Azure, and GCP do not publish a type string per service. Every icon in all
three is expressed as `namedShape` or `namedContainer` carrying a `className`,
and the class name encodes its vendor: `GCP2021…` by prefix, `…Azure2024` and
`…AWS2024` by suffix. That is what `CloudVendor::from_class_name` reads, and why
there are two shape types here rather than several hundred.

### Visio `.vsdx`

| Topic | Source |
|---|---|
| OPC container, parts, relationships | ECMA-376 Part 2, Open Packaging Conventions |
| Page index, page contents, masters, cell tables, `Connects` | `[MS-VSDX]`, Microsoft's published Visio Graphics Format |

The two conventions that matter most, and that the importer exists to undo, are
that Visio measures in inches from the **bottom left** with the y axis pointing
**up**, and that it positions a shape by its *pin* — the point it rotates about
— rather than by a corner. Both conversions live in `dc-vsdx`'s `sheet` module
and are covered by tests that state the expected numbers.

Where a published page states a default — black 1 px strokes, white fills, 10 pt
Liberation Sans, centered text, inches as the default unit — that default is
encoded in the model and covered by a test that names the source. See
`crates/dc-model/src/style.rs` and `crates/dc-model/src/document.rs`.

## What is *not* from those sources

Three categories of work are our own and are not reverse engineered from
anything:

**Shape outlines.** The published libraries name 68 shape types but do not
specify their geometry. The outlines in `crates/dc-geom/src/shapes.rs` were
drawn from the conventional public symbol vocabularies — ANSI/ISO 5807 for the
flowchart shapes, OMG's BPMN 2.0 notation for the BPMN ones, and plain regular
polygons elsewhere. They are not traced from another program's rendering.

**Master-name mapping.** A Visio shape names the stencil master it came from,
not a geometric type. The table in `crates/dc-vsdx/src/masters.rs` maps the
universal names of Microsoft's stock Basic Shapes and Basic Flowchart stencils
onto types this build draws. Those names are visible in any `.vsdx` file and in
Visio's own documentation; the mapping itself — which master should become
which outline — is a judgement call, and it is ours.

**The interface layout.** The menu bar follows the headings Lucidchart's own
help documentation describes for its top menu — File, Edit, Select, Insert,
Arrange, View, Help — because those headings are a reasonable taxonomy and
following them makes the application legible to someone arriving from that
product. The contents are this program's own commands, not a transcription:
commands Lucidchart has and this does not are absent rather than present and
dead, and the Format menu is an addition, because a desktop application should
show formatting shortcuts in its menu bar.

The properties panel is grouped the same way, and for the same reason: Lucid's
help pages enumerate what their right panel holds — fill, "line stroke and
border options (color, thickness, style, and endpoints)", object properties,
and arrangement — and say the bar "only shows the controls that are relevant
to what you have selected". Those are the headings and that behaviour. The
controls underneath are this program's own, drawn with this program's own
geometry; the convention of *showing* a line weight rather than naming it is
an idea, and ideas are not something anyone owns.

Nothing in this layout came from inspecting the running application. That
offer was declined precisely because it would have made the claim at the top
of this document false.

**Connector routing.** The format records a connector's endpoints and whether
it is straight, elbow, or curved. How an elbow route actually bends is left
entirely to the implementation. Our router generates candidate orthogonal
polylines, scores them against obstacles, bends, and length, and picks the best.
That algorithm is ours.

**The interface.** DiagramCraft's layout, palette, color scheme, and
interaction details are its own. We deliberately did not copy another product's
chrome, and the accent color is deliberately different. Format compatibility is
the goal; trade dress is not.

**Text editing.** Caret movement, selection, word boundaries, and input method
handling come from Parley's `PlainEditor`, an Apache/MIT library from the
Linebender project. The markup subset the format permits is parsed and
re-serialized by our own code, written from the published support table.

## Known limits of the published specification

Being honest about where the documentation stops:

- The lines reference states that 24 endpoint styles exist but enumerates 22.
  Those 22 are implemented; any other value is preserved verbatim through an
  `Other` variant rather than rejected.
- The layer object is referenced by the page spec but not detailed. `Layer` is
  modeled permissively: conventional fields are typed, everything else is kept.
- `elbowControlPoints` has no published shape. It is carried through untouched.
- The named page sizes are not published exhaustively. Known names resolve to
  their standard dimensions; unknown names render at US Letter while keeping
  their original spelling on save.
- The cloud libraries name their icons but do not ship the artwork, and there
  is no published geometry for any of them. A `namedShape` therefore draws as a
  tile carrying its vendor's mark and its own text. Every field, `className`
  included, survives a round trip.
- The Table library stores cell text in a `cells` array whose per-cell shape is
  not published in enough detail to lay out. The table's frame and rules are
  drawn from `rowCount` and `colCount`; `cells` is carried through untouched.
- The BPMN reference names each shape's `taskType`, `eventType`, and marker
  properties but not what any of them should look like. The glyphs drawn for
  them follow OMG's BPMN 2.0 notation, which is public.
- The markup subset is published as a support table rather than a grammar, so
  the parser is permissive by design: unknown tags are stripped while their
  content is kept, and unbalanced markup cannot make it fail.

## Fidelity guarantee

Loading and saving a document preserves it *semantically*: every field the
input carried appears in the output with the same value, including fields this
build does not interpret. This is enforced by `#[serde(flatten)]` capture on
`Shape`, `Line`, `Page`, `Group`, `Layer`, and `Document`, and tested in
`crates/dc-model/src/shape.rs` and `crates/dc-lucid/src/lib.rs`.

It is deliberately *not* byte-exact. JSON key order and number formatting carry
no meaning in this format, and pretending otherwise would be a false promise.

## Method

1. Read the published specification.
2. Write the Rust types and a test that quotes the specification's own example
   verbatim.
3. Make the test pass.

The published JSON examples appear verbatim in the test suite — see
`published_example_parses` in `crates/dc-model/src/line.rs`,
`documented_style_example_parses` in `style.rs`, and
`published_document_example_parses` in `document.rs`. If Lucid revises the
format, those tests are where the drift will surface first.

## On reverse engineering

Reverse-engineering tooling was considered and rejected as unnecessary. Lucid is
a web application with no local binary and no local document format to examine,
and the interchange format we needed is published. There was nothing to
decompile that would have told us anything the documentation did not.

The same applies to the menu bar. Working out what a competitor's menus contain
is a matter of reading its own help documentation, which describes them, and
that is what was done. No part of the application was instrumented, and no
screenshot was traced; the headings are a taxonomy, and taxonomies are not
something anyone owns.

This matters beyond principle: it means the provenance of every line in this
repository is a public URL.
