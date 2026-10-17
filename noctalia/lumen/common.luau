--!nonstrict
-- Shared by the bar icon and the menu. The lights are driven by ~/.local/bin/lumen, which talks to
-- them over the local network (miIO) and prints their state as JSON.
local M = {}

M.CMD = noctalia.expandPath("~/.local/bin/lumen")
M.STATE_KEY = "lumen.lights"
M.PANEL = "alecsh/lumen:menu"

-- the same presets as the lumen program (src/lights.rs)
M.COLORS = {
    { "Red", "#ff0000" }, { "Orange", "#ff6a00" }, { "Amber", "#ffa000" }, { "Yellow", "#ffe000" },
    { "Lime", "#9dff00" }, { "Green", "#00ff30" }, { "Mint", "#00ffa0" }, { "Cyan", "#00e5ff" },
    { "Sky", "#0090ff" }, { "Blue", "#0030ff" }, { "Indigo", "#4b00ff" }, { "Violet", "#9000ff" },
    { "Magenta", "#ff00e0" }, { "Pink", "#ff3090" }, { "Rose", "#ff6080" }, { "White", "#ffffff" },
}
M.WHITES = {
    { name = "Reading", glyph = "book", bright = 100, ct = 4000 },
    { name = "Computer", glyph = "device-desktop", bright = 60, ct = 5000 },
    { name = "Focus", glyph = "focus", bright = 100, ct = 6500 },
    { name = "Relax", glyph = "armchair", bright = 50, ct = 2700 },
    { name = "Night", glyph = "moon", bright = 1, ct = 2700 },
}
M.SCENES = {
    { name = "Candle", colors = { "#ff8a1c", "#ffb44a", "#ff7a10" } },
    { name = "Sunrise", colors = { "#ff4400", "#ffa040", "#fff0d0" } },
    { name = "Sunset", colors = { "#ffd090", "#ff8030", "#a02000" } },
    { name = "Romance", colors = { "#5915ed", "#ff1a6a", "#8a00c0" } },
    { name = "Ocean", colors = { "#0077ff", "#00ffaa", "#4b00ff" } },
    { name = "Lava", colors = { "#ff0000", "#ff5000", "#ffa000" } },
    { name = "Rainbow", colors = { "#ff0000", "#00ff00", "#0000ff" } },
    { name = "Party", colors = { "#ff00e0", "#00e5ff", "#ffe000" } },
    { name = "Police", colors = { "#ff0000", "#0030ff", "#ff0000" } },
    { name = "Breathe", colors = { "#0090ff", "#003070", "#0090ff" } },
}

local function decode(out)
    local ok, v = pcall(noctalia.json.decode, out or "")
    if ok and type(v) == "table" then return v end
    return nil
end

-- merge lights from a command's answer into the published list (a command prints only its targets)
local function publish(rows)
    if rows == nil then return end
    local cur = noctalia.state.get and noctalia.state.get(M.STATE_KEY) or nil
    if type(cur) ~= "table" or #cur == 0 or #rows >= #cur then
        noctalia.state.set(M.STATE_KEY, rows)
        return
    end
    local byAlias = {}
    for _, r in ipairs(rows) do byAlias[r.alias] = r end
    local merged = {}
    for _, l in ipairs(cur) do table.insert(merged, byAlias[l.alias] or l) end
    noctalia.state.set(M.STATE_KEY, merged)
end
M.publish = publish

function M.refresh(done)
    noctalia.runAsync({ M.CMD, "--json", "status" }, function(result)
        local rows = decode(result.stdout)
        publish(rows)
        if done then done(rows) end
    end)
end

-- One command at a time per key; while one runs, only the newest waiting one is kept
-- (a dragged slider sends many values).
local busy, waiting = {}, {}

function M.send(key, args, done)
    if busy[key] then
        waiting[key] = { args = args, done = done }
        return
    end
    busy[key] = true
    local argv = { M.CMD, "--json" }
    for _, a in ipairs(args) do table.insert(argv, tostring(a)) end
    noctalia.runAsync(argv, function(result)
        busy[key] = false
        local rows = decode(result.stdout)
        local nxt = waiting[key]
        waiting[key] = nil
        if nxt then
            M.send(key, nxt.args, nxt.done)   -- the newer value wins; skip publishing this one
        else
            publish(rows)
        end
        if done then done(result.exitCode == 0, rows) end
    end)
end

function M.anyOn(lights)
    for _, l in ipairs(lights or {}) do
        if l.online and l.power then return true end
    end
    return false
end

return M
