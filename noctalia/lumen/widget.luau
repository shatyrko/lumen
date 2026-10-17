--!nonstrict
-- Bar icon: a bulb, filled while any light is on (crossed out when none answers). Tooltip: each
-- light and what it does. Click: the menu. Scroll: brightness of the lights that are on.
-- Middle click: all lights on, or all off when any is on.
local g = require("./common.luau")

local lights = {}

local function show(v)
    if type(v) == "table" then lights = v end
    local online = 0
    for _, l in ipairs(lights) do
        if l.online then online += 1 end
    end
    local glyph = (#lights > 0 and online == 0) and "bulb-off" or (g.anyOn(lights) and "bulb-filled" or "bulb")
    barWidget.render(ui.glyph({ name = glyph }))
    local tip = {}
    for _, l in ipairs(lights) do
        table.insert(tip, { l.name, l.describe })
    end
    table.insert(tip, { "", "Click for the lights · scroll for brightness · middle click for all on/off" })
    barWidget.setTooltip(tip)
end

function update()
    g.refresh()
end

function onClick()
    noctalia.togglePanel(g.PANEL)
end

function onMiddleClick()
    g.send("power", { g.anyOn(lights) and "off" or "on" })
end

-- wheel up = brighter, 5 % a notch, for the lights that are on
function onScroll(axis, steps)
    if axis ~= "vertical" or steps == 0 then return end
    local on = {}
    for _, l in ipairs(lights) do
        if l.online and l.power then
            table.insert(on, l.alias)
            l.bright = math.clamp(l.bright - steps * 5, 1, 100)
        end
    end
    if #on == 0 then return end
    -- one absolute value per light keeps a fast scroll from piling up relative steps
    for _, l in ipairs(lights) do
        if l.online and l.power then
            g.send("bright-" .. l.alias, { "bright", l.bright, l.alias })
        end
    end
    show(lights)
end

noctalia.state.watch(g.STATE_KEY, show)
noctalia.setUpdateInterval(30000)   -- changes made elsewhere (Mi Home, a wall switch)
show({})
g.refresh()
