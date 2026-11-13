# Device geometry interface (#345)

Considered three shapes: an editor-owned geometry service, a new geometry module,
and pure functions extending `td_core::live`. Choose the last: existing Tile and
cover callers already cross this seam, and the native editor can use it without
UI dependencies or a parallel coordinate model. A service adds lifecycle without
state; a sibling module splits the existing public interface.

Screen sizes are oriented explicitly; Tile rectangles use Device pixels, while
Live content rectangles use Tile-local pixels. The inverse retains the preceding
cell through its following gap, including fractional pointer positions. Out of
grid indices are preserved for editor placement decisions. Invalid grids return
None. Now-playing returns art, text and control zones; bar returns its glyph,
track and optional interactive zones. These describe geometry, never drawing.
A now-playing Tile has no progress bar. Optional Integration values must not
create visual elements the Device does not draw (6546969). Asset preparation uses
the same art box through `art_size`; covers are centered and cropped, not stretched.
Shared JSON vectors remain in place for Node, Rust and Firmware consumers.
