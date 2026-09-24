import React, { Suspense, lazy } from "react";
import ReactDOM from "react-dom/client";
import "./styles/global.css";

const isPresentation = typeof window !== "undefined" && window.location.hash.startsWith("#presentation");

// Each window loads only its own bundle: the presentation window doesn't need
// the whole app (library, player, settings...) and vice versa.
const App = lazy(() => import("./App.jsx"));
const PresentationView = lazy(() => import("./views/PresentationView.jsx"));

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    <Suspense fallback={null}>
      {isPresentation ? <PresentationView /> : <App />}
    </Suspense>
  </React.StrictMode>
);
