# Model portraits

`SetPortraitTexture__524f60` renders 64 by 64 model portraits into single-level
`A8R8G8B8` D3D9 render-target textures. File portraits and animated model widgets
use their existing paths. No addon changes are required.

The native cache owns each texture handle. Players use their full GUID;
creatures share portraits by display ID. The replacement retains the native
model-readiness checks, scene clone, animation, camera and lighting helpers.
Unavailable models use the existing GUID-based deferred update queue.

The alpha mask comes from the client's `GetAlphaMask(64)`. It is uploaded once
per device into a managed texture. A point-sampled fullscreen draw writes only
alpha, with blending disabled, so mask bytes replace alpha without changing
RGB. The portrait target and depth surface have no multisampling. The pass
checks format compatibility and leaves an existing multiple-render-target pass
alone.

The native render-state stack restores application state. Projection, model
matrix, viewport, clear color and screen flags are also restored. The alpha
draw uses a D3D state block; render target, depth and hardware viewport have
separate scoped restoration. Publishing a new backend texture forces native
state application so bindings follow the unchanged texture handle.

`CGxDeviceD3d__ITexUpdate__5a0d70` recognizes GPU portrait records and bypasses
CPU uploads. `CGxDeviceD3d__TexDestroy__5a0850` removes validity metadata before
the native allocation is freed. Before device release, owned portrait targets
and auxiliary default-pool depth storage are released. Managed mask storage
survives reset and is released at device teardown. A new generation invalidates
cached GPU content and queues observed GUIDs for native refresh events. No
borrowed unit pointers survive the request.

All required hooks and native helper signatures must verify before activation.
A conflicting hook, including an existing device-release detour, disables the
feature. Unsupported graphics backends use stock behavior. A failed render or
publication retains an existing portrait when available and defers refresh.
Failures never invoke the stock readback renderer or publish cache validity.

## Validation

`make check` covers portable cache and ownership tests, mask-byte encoding, all
build configurations, documentation and repository audits. Native hook and
helper ABIs require separate checks against the supported client. The renderer
has no numeric differential harness: running the original and replacement
would render twice and mutate shared device state.

Build both shipped variants with `PROD=1 make windows` and
`PROD=1 make windows-avx`. An instrumented build uses `PERF=1`; its `portrait:`
line reports requests, cache hits, renders, regenerations, failures, total
duration and peak duration. Each request also logs its GUID, cache identity,
outcome and duration, so short runs retain diagnostic evidence.

Live acceptance is operator-driven. Compare fresh launches with the same
renderer configuration and camera/target sequence. The control launch uses
`WOW_TURBO_SKIP=SetPortraitTexture__524f60`; the other leaves that hook enabled.
Preserve both launch logs and their exact DLL/PDB pairs.

Check player and creature portraits, shared creature display IDs, appearance
changes, unloaded models, rapid targeting, UI reload, logout/login, resolution
changes and device reset. Portraits must recover after reset without retargeting,
with correct colors, alpha edges, orientation and sampling. Resource counts
must settle after repeated refresh/reset cycles.

A successful GPU portrait render must cause no back-buffer lock, CPU readback
or presentation snapshot. Compare frame-time spikes separately from visual
correctness. Portable tests and ABI checks cannot establish either live result.
