import React from "react";
import { Composition } from "remotion";
import { FPS, H, W } from "./lib";
import { Trailer, TOTAL } from "./Trailer";

export const Root: React.FC = () => (
  <Composition id="TrailerV2" component={Trailer} durationInFrames={Math.round(TOTAL * FPS)} fps={FPS} width={W} height={H} />
);
