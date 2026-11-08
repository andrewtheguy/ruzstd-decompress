// One thread of a decoder's pool: an instance of the module on the memory of the decoder
// worker's instance, which takes a seat in the pool and decodes what the pool hands it.
//
// It says once whether its instance is made, then takes its seat, which returns only when the
// decoder is freed; it says so then, to be ended.
import { initSync, runPoolThread } from "../pkg/ruzstd_mt.js";
import { ownerPort, reason } from "./env.js";

const owner = await ownerPort();
owner.onMessage(({ module, memory }) => {
  try {
    initSync({ module, memory });
  } catch (error) {
    owner.post({ type: "failed", reason: reason(error) });
    return;
  }
  owner.post({ type: "seated" });
  try {
    runPoolThread();
  } catch (error) {
    owner.post({ type: "failed", reason: reason(error) });
    return;
  }
  owner.post({ type: "stopped" });
});
