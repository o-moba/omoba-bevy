# Vulkan descriptor allocation on Mali

This is the published wgpu-hal 30.0.0 crate with the upstream allocator fix from [wgpu PR 10264](https://github.com/gfx-rs/wgpu/pull/10264) applied to `src/vulkan/descriptor.rs`. The Mali-G925 driver on PKB110 returned `VK_ERROR_FRAGMENTED_POOL` during the first frame. The fix retries in another pool and resets a fragmented pool once its sets have all been released.

Remove the local Cargo patch when the pinned renderer resolves to a wgpu-hal release containing upstream commit `573944a9712a91601c8ed508e615b23cd84a7489`.
