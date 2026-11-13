Superseded by ADR 0011.

# React for the UI

The UI is rebuilt in React with Vite instead of staying on Svelte 5. The overhaul rewrites every screen anyway, and the libraries and design skills we build with (base-ui, motion, Sonner, dnd kit, Liveline, zustand, cva) are React-only. The Device preview logic is plain TypeScript and moves over unchanged.
