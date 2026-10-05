/**
 * Motion graphic for an MDX guide.
 * A mark travels along each arrow. No extra dependency.
 * Pass nodes in reading order. An edge that returns to an earlier node
 * draws under the row.
 */
export function Flow({ label, nodes, edges, dir = "row" }) {
  const index = new Map(nodes.map((node, i) => [node.id, i]))
  const forward = edges.filter(([from, to]) => index.get(to) > index.get(from))
  const back = edges.filter(([from, to]) => index.get(to) <= index.get(from))

  const depth = new Map(nodes.map((node) => [node.id, 0]))
  let changed = true
  while (changed) {
    changed = false
    for (const [from, to] of forward) {
      const next = depth.get(from) + 1
      if (next > depth.get(to)) {
        depth.set(to, next)
        changed = true
      }
    }
  }

  const columns = new Map()
  for (const node of nodes) {
    const column = depth.get(node.id)
    const list = columns.get(column) ?? []
    list.push(node)
    columns.set(column, list)
  }
  const columnKeys = [...columns.keys()].sort((a, b) => a - b)
  const maxRows = Math.max(...columnKeys.map((key) => columns.get(key).length))

  const boxW = 148
  const boxH = 44
  const gapX = 56
  const gapY = 28
  const pad = 16
  const across = dir === "col" ? maxRows : columnKeys.length
  const down = dir === "col" ? columnKeys.length : maxRows
  const width = pad * 2 + across * boxW + (across - 1) * gapX
  const height = pad * 2 + down * boxH + (down - 1) * gapY + (back.length ? 28 : 0)

  const pos = new Map()
  columnKeys.forEach((column, columnIndex) => {
    const list = columns.get(column)
    list.forEach((node, row) => {
      const xIndex = dir === "col" ? row : columnIndex
      const yIndex = dir === "col" ? columnIndex : row
      const x = pad + xIndex * (boxW + gapX)
      const y = pad + yIndex * (boxH + gapY)
      pos.set(node.id, { x, y, cx: x + boxW / 2, cy: y + boxH / 2 })
    })
  })

  const edgePath = ([from, to], isBack) => {
    const a = pos.get(from)
    const b = pos.get(to)
    if (!a || !b) return ""
    if (isBack) {
      const y = height - 12
      return `M ${a.cx} ${a.y + boxH} V ${y} H ${b.cx} V ${b.y + boxH}`
    }
    if (dir === "col") {
      return `M ${a.cx} ${a.y + boxH} V ${b.y}`
    }
    return `M ${a.x + boxW} ${a.cy} H ${b.x}`
  }

  return (
    <figure className="elm-flow" aria-label={label} style={{ margin: "1rem 0" }}>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        width="100%"
        style={{ maxWidth: width, height: "auto", color: "inherit" }}
      >
        <title>{label}</title>
        <style>{`
          .elm-flow-edge {
            fill: none;
            stroke: #2563eb;
            stroke-width: 2;
            stroke-dasharray: 8 8;
            animation: elm-flow-move 1.1s linear infinite;
          }
          .elm-flow-box {
            fill: transparent;
            stroke: currentColor;
            stroke-width: 1.5;
          }
          .elm-flow-label {
            fill: currentColor;
            font: 13px ui-sans-serif, system-ui, sans-serif;
          }
          @keyframes elm-flow-move {
            to { stroke-dashoffset: -32; }
          }
          @media (prefers-reduced-motion: reduce) {
            .elm-flow-edge { animation: none; }
          }
        `}</style>
        {forward.map((edge) => (
          <path key={`${edge[0]}-${edge[1]}`} className="elm-flow-edge" d={edgePath(edge, false)} />
        ))}
        {back.map((edge) => (
          <path key={`${edge[0]}-${edge[1]}-back`} className="elm-flow-edge" d={edgePath(edge, true)} />
        ))}
        {nodes.map((node) => {
          const point = pos.get(node.id)
          return (
            <g key={node.id}>
              <rect className="elm-flow-box" x={point.x} y={point.y} width={boxW} height={boxH} rx="8" />
              <text
                className="elm-flow-label"
                x={point.cx}
                y={point.cy + 4}
                textAnchor="middle"
              >
                {node.label}
              </text>
            </g>
          )
        })}
      </svg>
    </figure>
  )
}
