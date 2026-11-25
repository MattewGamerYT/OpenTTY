import { useMemo, useState } from 'react';
import { Check, Copy, FileCode2, FolderTree, Search, Terminal as TerminalIcon } from 'lucide-react';

import cargoToml from '../Cargo.toml?raw';
import cargoConfigToml from '../.cargo/config.toml?raw';
import buildRs from '../build.rs?raw';
import glyphWgsl from '../shaders/glyph.wgsl?raw';
import readmeMd from '../README.md?raw';
import mainRs from './main.rs?raw';
import configRs from './config.rs?raw';
import ptyRs from './pty.rs?raw';
import fontModRs from './font/mod.rs?raw';
import termModRs from './terminal/mod.rs?raw';
import termColorRs from './terminal/color.rs?raw';
import termGridRs from './terminal/grid.rs?raw';
import termParserRs from './terminal/parser.rs?raw';
import waylandModRs from './wayland/mod.rs?raw';
import waylandInputRs from './wayland/input.rs?raw';
import vulkanModRs from './vulkan/mod.rs?raw';
import vulkanSwapchainRs from './vulkan/swapchain.rs?raw';
import vulkanPipelineRs from './vulkan/pipeline.rs?raw';
import vulkanAtlasRs from './vulkan/atlas.rs?raw';

interface SourceFile {
  path: string;
  subsystem: 'Build' | 'Shaders' | 'Core' | 'Wayland' | 'Vulkan' | 'Terminal';
  summary: string;
  content: string;
}

const SOURCE_FILES: SourceFile[] = [
  {
    path: 'Cargo.toml',
    subsystem: 'Build',
    summary: 'Rust edition 2024 manifest with ash, wayland-client, xkbcommon, nix, vte, and fontdue.',
    content: cargoToml,
  },
  {
    path: '.cargo/config.toml',
    subsystem: 'Build',
    summary: 'Native x86_64-unknown-linux-gnu CPU instruction tuning (-C target-cpu=native).',
    content: cargoConfigToml,
  },
  {
    path: 'build.rs',
    subsystem: 'Build',
    summary: 'Pure-Rust naga build script compiling WGSL into SPIR-V without external C++ tools.',
    content: buildRs,
  },
  {
    path: 'shaders/glyph.wgsl',
    subsystem: 'Shaders',
    summary: 'Single-draw-call instanced vertex & fragment shader for cells, glyphs, and cursor.',
    content: glyphWgsl,
  },
  {
    path: 'src/main.rs',
    subsystem: 'Core',
    summary: 'Zero-busy-loop poll event loop coordinating Wayland, Vulkan, xkbcommon, and Fish PTY.',
    content: mainRs,
  },
  {
    path: 'src/config.rs',
    subsystem: 'Core',
    summary: 'Terminal defaults: /usr/bin/fish, font fallbacks, padding, scrollback, and RGBA8 palette.',
    content: configRs,
  },
  {
    path: 'src/pty.rs',
    subsystem: 'Core',
    summary: 'Non-blocking openpty backend with xterm-256color/truecolor env and TIOCSWINSZ SIGWINCH.',
    content: ptyRs,
  },
  {
    path: 'src/font/mod.rs',
    subsystem: 'Core',
    summary: 'System monospace discovery via fontdb and fractional-scale rasterization via fontdue.',
    content: fontModRs,
  },
  {
    path: 'src/wayland/mod.rs',
    subsystem: 'Wayland',
    summary: 'Native Wayland client: KWin Breeze SSD, fractional scaling, viewporter, frame callbacks.',
    content: waylandModRs,
  },
  {
    path: 'src/wayland/input.rs',
    subsystem: 'Wayland',
    summary: 'xkbcommon keymap loading, repeat timer state, and CSI/SS3/UTF-8 key encoding.',
    content: waylandInputRs,
  },
  {
    path: 'src/vulkan/mod.rs',
    subsystem: 'Vulkan',
    summary: 'ash Vulkan renderer with VK_KHR_wayland_surface and single vkCmdDraw instancing.',
    content: vulkanModRs,
  },
  {
    path: 'src/vulkan/swapchain.rs',
    subsystem: 'Vulkan',
    summary: 'Swapchain manager prioritizing MAILBOX present mode and atomic resize rebuilds.',
    content: vulkanSwapchainRs,
  },
  {
    path: 'src/vulkan/pipeline.rs',
    subsystem: 'Vulkan',
    summary: 'Graphics pipeline with per-instance CellInstance attributes and dynamic viewport/scissor.',
    content: vulkanPipelineRs,
  },
  {
    path: 'src/vulkan/atlas.rs',
    subsystem: 'Vulkan',
    summary: '2048x2048 R8_UNORM shelf-packed glyph atlas with command-buffer staging uploads.',
    content: vulkanAtlasRs,
  },
  {
    path: 'src/terminal/mod.rs',
    subsystem: 'Terminal',
    summary: 'Primary/alternate screen manager and vte::Parser wrapper.',
    content: termModRs,
  },
  {
    path: 'src/terminal/grid.rs',
    subsystem: 'Terminal',
    summary: 'Scrollback ring buffer, DECSTBM scroll regions, cursor wrap, and grid resize.',
    content: termGridRs,
  },
  {
    path: 'src/terminal/parser.rs',
    subsystem: 'Terminal',
    summary: 'vte::Perform CSI, OSC title, SGR 256-color/truecolor, and DSR/DA1 replies.',
    content: termParserRs,
  },
  {
    path: 'src/terminal/color.rs',
    subsystem: 'Terminal',
    summary: 'xterm-256color palette and 24-bit RGB to packed RGBA8 resolution.',
    content: termColorRs,
  },
  {
    path: 'README.md',
    subsystem: 'Build',
    summary: 'CachyOS prerequisites and cargo run --release quickstart.',
    content: readmeMd,
  },
];

const SUBSYSTEMS = ['All', 'Build', 'Wayland', 'Vulkan', 'Terminal', 'Core', 'Shaders'] as const;

export default function App() {
  const [selectedPath, setSelectedPath] = useState<string>('src/main.rs');
  const [filterSubsystem, setFilterSubsystem] = useState<string>('All');
  const [searchQuery, setSearchQuery] = useState<string>('');
  const [copiedPath, setCopiedPath] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<'code' | 'architecture'>('code');

  const filteredFiles = useMemo(() => {
    const q = searchQuery.trim().toLowerCase();
    return SOURCE_FILES.filter((file) => {
      const matchesSubsystem =
        filterSubsystem === 'All' || file.subsystem === filterSubsystem;
      const matchesQuery =
        q.length === 0 ||
        file.path.toLowerCase().includes(q) ||
        file.summary.toLowerCase().includes(q);
      return matchesSubsystem && matchesQuery;
    });
  }, [filterSubsystem, searchQuery]);

  const activeFile =
    SOURCE_FILES.find((f) => f.path === selectedPath) ?? SOURCE_FILES[0];

  const totalLines = useMemo(
    () =>
      SOURCE_FILES.reduce(
        (acc, file) => acc + file.content.split('\n').length,
        0
      ),
    []
  );

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(activeFile.content);
      setCopiedPath(activeFile.path);
      window.setTimeout(() => setCopiedPath(null), 1600);
    } catch {
      // Clipboard API unavailable in restricted contexts
    }
  };

  return (
    <div className="min-h-screen bg-[#0B0F17] text-slate-100 flex flex-col">
      {/* Three-zone Top Bar */}
      <header className="border-b border-slate-800/90 bg-[#0F1521] px-6 py-3.5 flex items-center justify-between gap-8">
        <a
          href="#top"
          className="text-lg font-semibold tracking-tight text-slate-50 whitespace-nowrap shrink-0"
        >
          OpenTTY
        </a>

        <nav className="hidden md:flex items-center gap-6 text-sm font-medium text-slate-400">
          <button
            type="button"
            onClick={() => setViewMode('code')}
            className={`transition-colors whitespace-nowrap shrink-0 cursor-pointer ${
              viewMode === 'code'
                ? 'text-slate-100 underline underline-offset-8 decoration-sky-400'
                : 'hover:text-slate-200'
            }`}
          >
            Source Explorer
          </button>
          <button
            type="button"
            onClick={() => setViewMode('architecture')}
            className={`transition-colors whitespace-nowrap shrink-0 cursor-pointer ${
              viewMode === 'architecture'
                ? 'text-slate-100 underline underline-offset-8 decoration-sky-400'
                : 'hover:text-slate-200'
            }`}
          >
            Pipeline Architecture
          </button>
          <button
            type="button"
            onClick={() => {
              setViewMode('code');
              setSelectedPath('Cargo.toml');
            }}
            className="hover:text-slate-200 transition-colors whitespace-nowrap shrink-0 cursor-pointer"
          >
            Cargo Manifest
          </button>
          <button
            type="button"
            onClick={() => {
              setViewMode('code');
              setSelectedPath('shaders/glyph.wgsl');
            }}
            className="hover:text-slate-200 transition-colors whitespace-nowrap shrink-0 cursor-pointer"
          >
            WGSL Shader
          </button>
        </nav>

        <div className="shrink-0">
          <button
            type="button"
            onClick={handleCopy}
            className="inline-flex items-center gap-2 px-4 py-2 text-xs font-medium bg-sky-500 hover:bg-sky-400 text-slate-950 rounded-md transition-colors whitespace-nowrap shrink-0 cursor-pointer focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-sky-400"
          >
            {copiedPath === activeFile.path ? (
              <Check className="w-3.5 h-3.5" />
            ) : (
              <Copy className="w-3.5 h-3.5" />
            )}
            <span>{copiedPath === activeFile.path ? 'Copied File' : 'Copy Active File'}</span>
          </button>
        </div>
      </header>

      {/* Main Content */}
      {viewMode === 'code' ? (
        <div className="flex-1 grid grid-cols-1 lg:grid-cols-[300px_1fr] min-h-0">
          {/* Sidebar File Tree */}
          <aside className="border-b lg:border-b-0 lg:border-r border-slate-800/90 bg-[#0D121C] flex flex-col">
            <div className="p-4 border-b border-slate-800/80 space-y-3">
              <div className="flex items-center justify-between text-xs text-slate-400">
                <span className="font-medium text-slate-200 flex items-center gap-2">
                  <FolderTree className="w-3.5 h-3.5 text-sky-400" />
                  <span>Workspace Files</span>
                </span>
                <span className="font-mono tabular-nums">
                  {SOURCE_FILES.length} files · {totalLines} lines
                </span>
              </div>

              <div className="relative">
                <Search className="w-3.5 h-3.5 text-slate-500 absolute left-3 top-1/2 -translate-y-1/2" />
                <input
                  type="text"
                  value={searchQuery}
                  onChange={(e) => setSearchQuery(e.target.value)}
                  placeholder="Filter path or module..."
                  className="w-full bg-[#131A27] border border-slate-800 rounded-md pl-8 pr-3 py-1.5 text-xs text-slate-200 placeholder:text-slate-500 focus:outline-none focus:border-sky-500"
                />
              </div>

              <div className="flex flex-wrap gap-1">
                {SUBSYSTEMS.map((sub) => (
                  <button
                    key={sub}
                    type="button"
                    onClick={() => setFilterSubsystem(sub)}
                    className={`px-2 py-1 text-[11px] font-medium rounded transition-colors cursor-pointer whitespace-nowrap shrink-0 ${
                      filterSubsystem === sub
                        ? 'bg-slate-800 text-sky-300'
                        : 'text-slate-400 hover:text-slate-200 hover:bg-slate-800/50'
                    }`}
                  >
                    {sub}
                  </button>
                ))}
              </div>
            </div>

            <div className="flex-1 overflow-y-auto p-2 space-y-0.5 max-h-[420px] lg:max-h-none">
              {filteredFiles.length === 0 ? (
                <div className="p-4 text-xs text-slate-500 text-center">
                  No files match "{searchQuery}".
                </div>
              ) : (
                filteredFiles.map((file) => {
                  const active = file.path === activeFile.path;
                  const lineCount = file.content.split('\n').length;
                  return (
                    <button
                      key={file.path}
                      type="button"
                      onClick={() => setSelectedPath(file.path)}
                      className={`w-full text-left px-3 py-2 rounded-md flex items-center justify-between gap-2 text-xs transition-colors cursor-pointer ${
                        active
                          ? 'bg-slate-800/90 text-slate-50'
                          : 'text-slate-400 hover:bg-slate-800/40 hover:text-slate-200'
                      }`}
                    >
                      <span className="font-mono truncate">{file.path}</span>
                      <span className="font-mono text-[11px] text-slate-500 tabular-nums shrink-0">
                        {lineCount}L
                      </span>
                    </button>
                  );
                })
              )}
            </div>
          </aside>

          {/* Code Viewport */}
          <main className="flex flex-col min-w-0 bg-[#0B0F17]">
            <div className="px-6 py-4 border-b border-slate-800/80 flex flex-col sm:flex-row sm:items-center sm:justify-between gap-2 bg-[#0E1420]">
              <div className="min-w-0">
                <div className="flex items-center gap-2.5 text-sm font-semibold text-slate-100">
                  <FileCode2 className="w-4 h-4 text-sky-400 shrink-0" />
                  <span className="font-mono truncate">{activeFile.path}</span>
                </div>
                <p className="text-xs text-slate-400 mt-1">{activeFile.summary}</p>
              </div>
              <div className="flex items-center gap-2 text-xs text-slate-400 shrink-0 font-mono tabular-nums">
                <span>{activeFile.subsystem}</span>
                <span aria-hidden="true">·</span>
                <span>Rust 2024</span>
                <span aria-hidden="true">·</span>
                <span>{activeFile.content.split('\n').length} lines</span>
              </div>
            </div>

            <div className="flex-1 overflow-auto p-6">
              <pre className="text-[13px] leading-6 font-mono text-slate-200 bg-[#0F1521] border border-slate-800/90 rounded-lg p-5 overflow-x-auto">
                <code>{activeFile.content}</code>
              </pre>
            </div>
          </main>
        </div>
      ) : (
        <main className="flex-1 max-w-6xl w-full mx-auto px-6 py-10 space-y-10">
          <section className="space-y-3 max-w-3xl">
            <div className="flex items-center gap-2 text-xs text-slate-400">
              <span>CachyOS (x86_64)</span>
              <span aria-hidden="true">·</span>
              <span>KDE Plasma KWin Wayland</span>
              <span aria-hidden="true">·</span>
              <span>Vulkan 1.2 + Fish Shell</span>
            </div>
            <h1 className="text-3xl font-semibold tracking-tight text-slate-50 text-balance">
              OpenTTY Step 1 System Architecture
            </h1>
            <p className="text-sm text-slate-300 leading-relaxed">
              Every frame flows from kernel-level FD readiness through the VT state machine into a single instanced Vulkan draw call paced exclusively by Wayland compositor frame callbacks.
            </p>
          </section>

          <section className="grid grid-cols-1 md:grid-cols-2 gap-6">
            <div className="border border-slate-800 bg-[#0F1521] rounded-lg p-6 space-y-3">
              <h2 className="text-base font-semibold text-slate-100">
                01. Native Wayland & KWin Integration
              </h2>
              <p className="text-sm text-slate-400 leading-relaxed">
                Uses <span className="font-mono text-slate-200">wayland-client</span> with <span className="font-mono text-slate-200">zxdg_decoration_manager_v1</span> for Breeze server-side titlebars, <span className="font-mono text-slate-200">wp_fractional_scale_v1</span> for crisp HiDPI scaling, and <span className="font-mono text-slate-200">wp_viewporter</span> so logical coordinates and physical swapchain pixels stay decoupled during atomic resizes.
              </p>
            </div>

            <div className="border border-slate-800 bg-[#0F1521] rounded-lg p-6 space-y-3">
              <h2 className="text-base font-semibold text-slate-100">
                02. Frame-Paced Vulkan Instancing
              </h2>
              <p className="text-sm text-slate-400 leading-relaxed">
                Creates <span className="font-mono text-slate-200">VK_KHR_wayland_surface</span> with zero X11/OpenGL dependencies, selects <span className="font-mono text-slate-200">MAILBOX</span> present mode, packs newly rasterized glyphs into an <span className="font-mono text-slate-200">R8_UNORM</span> atlas, and renders the entire terminal grid in one <span className="font-mono text-slate-200">vkCmdDraw(6, cells, 0, 0)</span>.
              </p>
            </div>

            <div className="border border-slate-800 bg-[#0F1521] rounded-lg p-6 space-y-3">
              <h2 className="text-base font-semibold text-slate-100">
                03. Zero-Busy-Loop Event Multiplexing
              </h2>
              <p className="text-sm text-slate-400 leading-relaxed">
                <span className="font-mono text-slate-200">nix::poll</span> sleeps in the kernel watching both the Wayland socket and the non-blocking PTY master FD, waking only on input, compositor events, shell output, or active <span className="font-mono text-slate-200">xkbcommon</span> key repeat intervals.
              </p>
            </div>

            <div className="border border-slate-800 bg-[#0F1521] rounded-lg p-6 space-y-3">
              <h2 className="text-base font-semibold text-slate-100">
                04. Fish PTY & Truecolor VT Engine
              </h2>
              <p className="text-sm text-slate-400 leading-relaxed">
                Spawns <span className="font-mono text-slate-200">/usr/bin/fish</span> in a new session with <span className="font-mono text-slate-200">TERM=xterm-256color</span> and <span className="font-mono text-slate-200">COLORTERM=truecolor</span>, forwards <span className="font-mono text-slate-200">TIOCSWINSZ</span> (<span className="font-mono text-slate-200">SIGWINCH</span>) on every grid change, and parses ANSI/CSI/OSC sequences via <span className="font-mono text-slate-200">vte</span>.
              </p>
            </div>
          </section>

          <section className="border border-slate-800 bg-[#0F1521] rounded-lg p-6 space-y-4">
            <div className="flex items-center gap-2 text-sm font-semibold text-slate-100">
              <TerminalIcon className="w-4 h-4 text-sky-400" />
              <span>Build & Run on CachyOS</span>
            </div>
            <pre className="text-xs font-mono text-slate-200 bg-[#0B0F17] border border-slate-800/80 rounded-md p-4 overflow-x-auto leading-6">
{`sudo pacman -S rustup vulkan-icd-loader wayland libxkbcommon
cargo run --release`}
            </pre>
          </section>
        </main>
      )}
    </div>
  );
}
