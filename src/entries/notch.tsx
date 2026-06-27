/* @refresh reload */
import { render } from "solid-js/web";
import Notch from "../views/Notch";
// The notch uses its OWN stylesheet — never the app/pill `styles.css`. It is
// always dark (a black panel that blends with the physical notch), so unlike
// every other window it deliberately does NOT apply or follow the app's
// light/dark appearance. See `src/notch.css`.
import "../notch.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root element not found");
}
render(() => <Notch />, root);
