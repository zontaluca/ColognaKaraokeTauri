import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App.jsx";
import PresentationView from "./views/PresentationView.jsx";
import "./styles/global.css";

const isPresentation = typeof window !== "undefined" && window.location.hash.startsWith("#presentation");

ReactDOM.createRoot(document.getElementById("root")).render(
  <React.StrictMode>
    {isPresentation ? <PresentationView /> : <App />}
  </React.StrictMode>
);
