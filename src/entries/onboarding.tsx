/* @refresh reload */
import { render } from "solid-js/web";
import Onboarding from "../views/Onboarding";
import "../styles.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("#root element not found");
}
render(() => <Onboarding />, root);
