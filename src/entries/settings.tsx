/* @refresh reload */
import { render } from "solid-js/web";
import Settings from "../views/Settings";
import "../styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root element not found");
}
render(() => <Settings />, root);
