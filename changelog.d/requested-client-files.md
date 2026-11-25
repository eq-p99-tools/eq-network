- Measure installed Titanium integrity files only when requested, with
  cancellable hashing that keeps transport serviced and reports numeric progress.
  Public checks use current file bytes; explicitly local-only EQEmu checks may
  reuse a size/mtime cache and invalidate refused challenge operands.
