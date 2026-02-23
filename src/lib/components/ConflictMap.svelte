<script lang="ts">
  import { getConflicts } from "$lib/api";
  import { selectedGame } from "$lib/stores";
  import type { FileConflict } from "$lib/types";

  interface Props {
    visible?: boolean;
    onclose?: () => void;
  }

  let { visible = true, onclose }: Props = $props();

  let conflicts = $state<FileConflict[]>([]);
  let loading = $state(false);

  const game = $derived($selectedGame);

  // Build graph data from conflicts
  interface GraphNode {
    id: number;
    name: string;
    priority: number;
    conflictCount: number;
    x: number;
    y: number;
    vx: number;
    vy: number;
    isWinner: boolean;
  }

  interface GraphEdge {
    source: number;
    target: number;
    fileCount: number;
    files: string[];
  }

  let nodes = $state<GraphNode[]>([]);
  let edges = $state<GraphEdge[]>([]);
  let hoveredNode = $state<number | null>(null);
  let hoveredEdge = $state<{ source: number; target: number } | null>(null);
  let selectedNode = $state<number | null>(null);
  let animationFrame = $state(0);
  let canvasWidth = $state(600);
  let canvasHeight = $state(400);

  $effect(() => {
    if (game && visible) {
      loadConflicts();
    }
  });

  async function loadConflicts() {
    if (!game) return;
    loading = true;
    try {
      conflicts = await getConflicts(game.game_id, game.bottle_name);
      buildGraph();
      runSimulation();
    } catch {
      conflicts = [];
    } finally {
      loading = false;
    }
  }

  function buildGraph() {
    const nodeMap = new Map<number, GraphNode>();
    const edgeMap = new Map<string, GraphEdge>();

    // Build nodes from all mods involved in conflicts
    for (const conflict of conflicts) {
      for (const mod of conflict.mods) {
        if (!nodeMap.has(mod.mod_id)) {
          nodeMap.set(mod.mod_id, {
            id: mod.mod_id,
            name: mod.mod_name,
            priority: mod.priority,
            conflictCount: 0,
            x: canvasWidth / 2 + (Math.random() - 0.5) * 200,
            y: canvasHeight / 2 + (Math.random() - 0.5) * 200,
            vx: 0,
            vy: 0,
            isWinner: false,
          });
        }
        const node = nodeMap.get(mod.mod_id)!;
        node.conflictCount++;
        if (mod.mod_id === conflict.winner_mod_id) {
          node.isWinner = true;
        }
      }

      // Build edges between all pairs of mods in each conflict
      for (let i = 0; i < conflict.mods.length; i++) {
        for (let j = i + 1; j < conflict.mods.length; j++) {
          const a = Math.min(conflict.mods[i].mod_id, conflict.mods[j].mod_id);
          const b = Math.max(conflict.mods[i].mod_id, conflict.mods[j].mod_id);
          const key = `${a}-${b}`;
          if (!edgeMap.has(key)) {
            edgeMap.set(key, { source: a, target: b, fileCount: 0, files: [] });
          }
          const edge = edgeMap.get(key)!;
          edge.fileCount++;
          if (edge.files.length < 5) {
            edge.files.push(conflict.relative_path.split("/").pop() || conflict.relative_path);
          }
        }
      }
    }

    nodes = Array.from(nodeMap.values());
    edges = Array.from(edgeMap.values());
  }

  function runSimulation() {
    let iterations = 0;
    const maxIterations = 120;

    function tick() {
      if (iterations >= maxIterations || !visible) return;
      iterations++;

      const centerX = canvasWidth / 2;
      const centerY = canvasHeight / 2;

      // Reset velocities
      for (const node of nodes) {
        node.vx = 0;
        node.vy = 0;
      }

      // Repulsion between all nodes
      for (let i = 0; i < nodes.length; i++) {
        for (let j = i + 1; j < nodes.length; j++) {
          const dx = nodes[j].x - nodes[i].x;
          const dy = nodes[j].y - nodes[i].y;
          const dist = Math.sqrt(dx * dx + dy * dy) || 1;
          const force = 3000 / (dist * dist);
          const fx = (dx / dist) * force;
          const fy = (dy / dist) * force;
          nodes[i].vx -= fx;
          nodes[i].vy -= fy;
          nodes[j].vx += fx;
          nodes[j].vy += fy;
        }
      }

      // Attraction along edges
      for (const edge of edges) {
        const source = nodes.find((n) => n.id === edge.source);
        const target = nodes.find((n) => n.id === edge.target);
        if (!source || !target) continue;
        const dx = target.x - source.x;
        const dy = target.y - source.y;
        const dist = Math.sqrt(dx * dx + dy * dy) || 1;
        const force = dist * 0.01;
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        source.vx += fx;
        source.vy += fy;
        target.vx -= fx;
        target.vy -= fy;
      }

      // Center gravity
      for (const node of nodes) {
        node.vx += (centerX - node.x) * 0.005;
        node.vy += (centerY - node.y) * 0.005;
      }

      // Apply velocities with damping
      const damping = 0.85;
      for (const node of nodes) {
        node.vx *= damping;
        node.vy *= damping;
        node.x += node.vx;
        node.y += node.vy;
        // Keep in bounds
        node.x = Math.max(40, Math.min(canvasWidth - 40, node.x));
        node.y = Math.max(40, Math.min(canvasHeight - 40, node.y));
      }

      animationFrame++;
      requestAnimationFrame(tick);
    }

    requestAnimationFrame(tick);
  }

  function getNodeRadius(node: GraphNode): number {
    return Math.min(24, 10 + node.conflictCount * 2);
  }

  function getEdgeWidth(edge: GraphEdge): number {
    return Math.min(6, 1 + edge.fileCount * 0.5);
  }

  function getNodeColor(node: GraphNode): string {
    if (selectedNode === node.id) return "var(--accent, #d98f40)";
    if (hoveredNode === node.id) return "var(--blue, #0a84ff)";
    if (node.isWinner) return "var(--green, #30d158)";
    return "var(--text-secondary, #8e8e93)";
  }

  function getEdgeColor(edge: GraphEdge): string {
    if (hoveredEdge && hoveredEdge.source === edge.source && hoveredEdge.target === edge.target) {
      return "var(--accent, #d98f40)";
    }
    if (selectedNode && (edge.source === selectedNode || edge.target === selectedNode)) {
      return "var(--blue, #0a84ff)";
    }
    if (edge.fileCount > 10) return "var(--red, #ff3b30)";
    if (edge.fileCount > 3) return "var(--orange, #ff9500)";
    return "var(--text-tertiary, #636366)";
  }

  // Tooltip
  const tooltip = $derived.by(() => {
    if (hoveredNode !== null) {
      const node = nodes.find((n) => n.id === hoveredNode);
      if (node) {
        const nodeEdges = edges.filter((e) => e.source === node.id || e.target === node.id);
        const totalFiles = nodeEdges.reduce((s, e) => s + e.fileCount, 0);
        return `${node.name}\nPriority: ${node.priority}\n${totalFiles} conflicting file${totalFiles !== 1 ? "s" : ""} across ${nodeEdges.length} mod${nodeEdges.length !== 1 ? "s" : ""}`;
      }
    }
    if (hoveredEdge) {
      const edge = edges.find(
        (e) => e.source === hoveredEdge!.source && e.target === hoveredEdge!.target,
      );
      if (edge) {
        const sName = nodes.find((n) => n.id === edge.source)?.name || "?";
        const tName = nodes.find((n) => n.id === edge.target)?.name || "?";
        return `${sName} \u2194 ${tName}\n${edge.fileCount} shared file${edge.fileCount !== 1 ? "s" : ""}${edge.files.length > 0 ? "\n" + edge.files.join(", ") + (edge.fileCount > 5 ? "..." : "") : ""}`;
      }
    }
    return null;
  });
</script>

{#if visible}
  <div class="conflict-map">
    <div class="map-header">
      <h3 class="map-title">Conflict Map</h3>
      <div class="map-legend">
        <span class="legend-item">
          <span class="legend-dot" style="background: var(--green, #30d158)"></span>
          Winner
        </span>
        <span class="legend-item">
          <span class="legend-dot" style="background: var(--text-secondary, #8e8e93)"></span>
          Overridden
        </span>
        <span class="legend-item">
          <span class="legend-line" style="background: var(--red, #ff3b30)"></span>
          Many conflicts
        </span>
      </div>
      {#if onclose}
        <button class="map-close" onclick={onclose} type="button">&times;</button>
      {/if}
    </div>

    {#if loading}
      <div class="map-loading">Analyzing conflicts...</div>
    {:else if nodes.length === 0}
      <div class="map-empty">No file conflicts detected</div>
    {:else}
      <div class="map-stats">
        {nodes.length} mod{nodes.length !== 1 ? "s" : ""} &middot;
        {edges.length} conflict relationship{edges.length !== 1 ? "s" : ""} &middot;
        {conflicts.length} file{conflicts.length !== 1 ? "s" : ""}
      </div>
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <svg
        class="map-svg"
        viewBox="0 0 {canvasWidth} {canvasHeight}"
        xmlns="http://www.w3.org/2000/svg"
      >
        {#key animationFrame}
          <!-- Edges -->
          {#each edges as edge (edge.source + "-" + edge.target)}
            {@const source = nodes.find((n) => n.id === edge.source)}
            {@const target = nodes.find((n) => n.id === edge.target)}
            {#if source && target}
              <!-- svelte-ignore a11y_no_static_element_interactions -->
              <line
                x1={source.x}
                y1={source.y}
                x2={target.x}
                y2={target.y}
                stroke={getEdgeColor(edge)}
                stroke-width={getEdgeWidth(edge)}
                stroke-opacity="0.6"
                onmouseenter={() => (hoveredEdge = { source: edge.source, target: edge.target })}
                onmouseleave={() => (hoveredEdge = null)}
              />
            {/if}
          {/each}

          <!-- Nodes -->
          {#each nodes as node (node.id)}
            {@const r = getNodeRadius(node)}
            <!-- svelte-ignore a11y_no_static_element_interactions -->
            <g
              onmouseenter={() => (hoveredNode = node.id)}
              onmouseleave={() => (hoveredNode = null)}
              onclick={() => (selectedNode = selectedNode === node.id ? null : node.id)}
              style="cursor: pointer"
            >
              <circle
                cx={node.x}
                cy={node.y}
                r={r}
                fill={getNodeColor(node)}
                fill-opacity="0.2"
                stroke={getNodeColor(node)}
                stroke-width="2"
              />
              <text
                x={node.x}
                y={node.y + r + 12}
                text-anchor="middle"
                fill="var(--text-secondary)"
                font-size="10"
                font-family="var(--font-body, -apple-system, BlinkMacSystemFont, sans-serif)"
              >
                {node.name.length > 16 ? node.name.slice(0, 15) + "\u2026" : node.name}
              </text>
              <text
                x={node.x}
                y={node.y + 4}
                text-anchor="middle"
                fill="var(--text-primary)"
                font-size="11"
                font-weight="600"
                font-family="var(--font-body, -apple-system, BlinkMacSystemFont, sans-serif)"
              >
                {node.conflictCount}
              </text>
            </g>
          {/each}
        {/key}
      </svg>

      {#if tooltip}
        <div class="map-tooltip">
          {#each tooltip.split("\n") as line}
            <div>{line}</div>
          {/each}
        </div>
      {/if}
    {/if}
  </div>
{/if}

<style>
  .conflict-map {
    background: var(--bg-grouped-secondary, #2c2c2e);
    border-radius: var(--radius-lg, 12px);
    overflow: hidden;
    position: relative;
  }

  .map-header {
    display: flex;
    align-items: center;
    gap: var(--space-3, 12px);
    padding: var(--space-3, 12px) var(--space-4, 16px);
    border-bottom: 1px solid var(--separator, #38383a);
  }

  .map-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text-primary);
    margin: 0;
  }

  .map-legend {
    display: flex;
    gap: var(--space-3, 12px);
    margin-left: auto;
    font-size: 11px;
    color: var(--text-tertiary);
  }

  .legend-item {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .legend-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }

  .legend-line {
    width: 16px;
    height: 3px;
    border-radius: 2px;
  }

  .map-close {
    background: none;
    border: none;
    color: var(--text-tertiary);
    font-size: 18px;
    cursor: pointer;
    padding: 0 4px;
    line-height: 1;
  }

  .map-close:hover {
    color: var(--text-primary);
  }

  .map-stats {
    font-size: 11px;
    color: var(--text-tertiary);
    padding: var(--space-2, 8px) var(--space-4, 16px) 0;
  }

  .map-svg {
    width: 100%;
    height: 350px;
    display: block;
  }

  .map-loading,
  .map-empty {
    padding: var(--space-6, 24px);
    text-align: center;
    color: var(--text-tertiary);
    font-size: 13px;
  }

  .map-tooltip {
    position: absolute;
    bottom: var(--space-3, 12px);
    left: var(--space-4, 16px);
    background: var(--bg-grouped-tertiary, #3a3a3c);
    border: 1px solid var(--separator, #38383a);
    border-radius: var(--radius-sm, 6px);
    padding: var(--space-2, 8px) var(--space-3, 12px);
    font-size: 11px;
    color: var(--text-secondary);
    pointer-events: none;
    max-width: 300px;
    line-height: 1.4;
    white-space: pre-wrap;
  }
</style>
