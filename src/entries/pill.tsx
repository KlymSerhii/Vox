/* @refresh reload */
import { render } from "solid-js/web";
import Pill from "../views/Pill";
import { getPublicSettings, onSettingsChanged } from "../lib/ipc";
import { applyTheme } from "../lib/theme";
import "../styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root element not found");
}
render(() => <Pill />, root);

void getPublicSettings()
  .then((s) => applyTheme(s.appearance))
  .catch(() => {});
void onSettingsChanged((s) => applyTheme(s.appearance));
