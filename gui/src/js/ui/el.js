// The DOM builder (F4). Moved out of the v1 components.js, which this commit deletes, into its own
// module — IMPLEMENTATION-PLAN.md §3.4. It is the one thing from the v1 layer that survives
// unchanged, because it is not styling: it is how every module in gui/src builds nodes.
//
// Deliberately NOT extended to SVG. `document.createElement("svg")` yields an HTML-namespace
// element that inspects correctly and renders nothing, and the `class` branch below assigns to
// `node.className`, which is a readonly SVGAnimatedString on a real SVGElement. ui/hexagon.js ships
// its own createElementNS builder for that reason; see its comment.

/** Tiny DOM builder. `props.class`, `on<Event>` handlers, boolean/scalar attrs; children flattened. */
export function el(tag, props = {}, ...children) {
  const node = document.createElement(tag);
  for (const [key, val] of Object.entries(props || {})) {
    if (key === "class") node.className = val;
    else if (key.startsWith("on") && typeof val === "function")
      node.addEventListener(key.slice(2).toLowerCase(), val);
    else if (val === true) node.setAttribute(key, "");
    else if (val !== false && val != null) node.setAttribute(key, String(val));
  }
  for (const child of children.flat()) {
    if (child == null || child === false) continue;
    node.append(child.nodeType ? child : document.createTextNode(String(child)));
  }
  return node;
}
