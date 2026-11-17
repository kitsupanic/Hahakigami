import { mount } from "svelte";
import App from "./App.svelte";
import "@fontsource-variable/archivo/wdth.css";
import {
  applyAppearance,
  applyGuardian,
  appearanceStored,
  guardianStored,
} from "./theme.svelte";
import "./tokens.css";
import "./style.css";
import "./native.css";
import "./workspace.css";
import "./controls.css";
applyGuardian(guardianStored());
applyAppearance(appearanceStored());
mount(App, { target: document.getElementById("root")! });
