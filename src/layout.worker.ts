import { layout } from "./layout";
self.onmessage = (event) => {
  const { requestId, nodes, rootId, width, height, mode, colorMode } =
    event.data;
  self.postMessage({
    requestId,
    shapes: layout(nodes, rootId, width, height, mode, colorMode),
  });
};
