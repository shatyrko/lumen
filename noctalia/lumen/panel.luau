--!nonstrict
-- The menu under the bar icon: one card per light with its switch and brightness. One card at a
-- time opens further:
--   Colour  (bulb, strip)  16 colour presets, "Any colour…" (Noctalia's colour picker)
--   White                  colour temperature (the desk lamp 2600-5000 K, the others 1700-6500 K)
--                          and the presets Reading, Computer, Focus, Relax, Night
--   Scenes  (bulb, strip)  Candle, Sunrise, Sunset, Romance, Ocean, Lava, Rainbow, Party, Police,
--                          Breathe, and Stop while one runs
-- Every change shows at once and is sent by ~/.local/bin/lumen in the background; what the
-- lights answer replaces the guess.
local g = require("./common.luau")
local fit = require("./fit.luau")
local box = fit.new({ min = 160 })

local lights = {}
local openAlias = nil        -- the card that shows its colour / white / scene controls
local tabOf = {}             -- alias -> "colour" | "white" | "scenes" (chosen by hand)
local isOpen = false
local searching = false
local render

local CARD = { gap = 8, padding = 12, radius = 12, fill = "surface_variant", align = "stretch" }

-- ----- sizes (logical px), for panel.setHeight -----
-- The menu is as tall as what it shows: the closed cards, the open card's current tab, a line per light
-- that is offline; a tab change or another card glides to the new height (fit.luau, kit patches
-- plugin-panel-height + plugin-panel-live-height). It scrolls only if that would not fit the screen.
-- measured on screen (2026-10-18, white + host again 2026-10-25: 24 px under the last card, as at the sides);
-- host = Noctalia's own padding around a plugin panel
local H = { pad = 12, header = 34, gap = 10, card = 108, offline = 44, tabs = 47, colour = 126, white = 108, scenes = 106, stop = 40, host = 24 }

local function find(alias)
    for _, l in ipairs(lights) do
        if l.alias == alias then return l end
    end
    return nil
end

local function tabFor(l)
    if tabOf[l.alias] then return tabOf[l.alias] end
    if l.flowing and l.can_scenes then return "scenes" end
    if (l.mode == "color" or l.mode == "hsv") and l.can_color then return "colour" end
    return "white"
end

-- what a light's card adds below its brightness row right now
local function extraOf(l)
    if not l.online then return H.offline end
    if openAlias ~= l.alias then return 0 end
    local t = tabFor(l)
    local h = (l.can_color or l.can_scenes) and H.tabs or 0
    if t == "colour" then return h + H.colour end
    if t == "scenes" then return h + H.scenes + (l.flowing and H.stop or 0) end
    return h + H.white
end

local function height()
    if #lights == 0 then return 160 end
    local h = H.pad * 2 + H.header + H.host
    for _, l in ipairs(lights) do
        h += H.gap + H.card + extraOf(l)
    end
    return h
end

-- closed (preload): the next open starts at the right height; open: glide there
local function setHeight()
    if isOpen then
        box.fit(height())
    elseif panel.setHeight then
        box.reset()
        panel.setHeight(height())
    end
end

-- ----- sending -----
-- `apply` changes the shown state at once; lumen's answer replaces it
local function act(l, key, args, apply)
    if apply then apply(l) end
    render()
    g.send(key .. "-" .. l.alias, args)
end

local function hexOf(rgb)
    return string.lower(rgb or "#ffffff")
end

-- ----- pieces -----
local function dot(l)
    local on = l.online and l.power
    return ui.box({
        width = 18, height = 18, radius = 9,
        fill = on and l.shown or "on_surface_variant/0.18",
        border = on and "on_surface/0.25" or "on_surface_variant/0.5", borderWidth = 1,
    })
end

local function segmented(key, items)
    local buttons = {}
    for i, it in ipairs(items) do
        table.insert(buttons, ui.button({
            key = key .. "-" .. i,
            text = it.text,
            glyph = it.glyph,
            variant = it.selected and "primary" or "ghost",
            selected = it.selected,
            flexGrow = 1,
            controlSize = "sm",
            onClick = it.onClick,
        }))
    end
    return ui.row({ key = key, gap = 4, align = "stretch", padding = 4, radius = 10, fill = "surface" }, buttons)
end

local function colourPage(l)
    local active = (l.mode == "color" and not l.flowing) and hexOf(l.rgb) or nil
    local rows = {}
    for r = 0, 1 do
        local cells = {}
        for c = 1, 8 do
            local entry = g.COLORS[r * 8 + c]
            local name, hex = entry[1], entry[2]
            local isActive = active == hex
            table.insert(cells, ui.box({
                key = `sw-{l.alias}-{name}`,
                width = 30, height = 30, radius = 15,
                fill = hex,
                border = isActive and "on_surface" or "on_surface/0.15",
                borderWidth = isActive and 3 or 1,
                tooltip = name,
                onClick = function()
                    act(l, "colour", { "color", hex, l.alias }, function(x)
                        x.power, x.mode, x.rgb, x.shown, x.flowing, x.scene = true, "color", hex, hex, false, nil
                        x.describe = `On · {x.bright}% · {name}`
                    end)
                end,
            }))
        end
        table.insert(rows, ui.row({ key = `swr-{l.alias}-{r}`, gap = 0, align = "center", justify = "space_between" }, cells))
    end
    table.insert(rows, ui.row({ key = `pick-{l.alias}`, gap = 8, align = "center" }, {
        ui.button({
            key = `pickb-{l.alias}`, glyph = "color-picker", text = "Any colour…", variant = "secondary", controlSize = "sm",
            onClick = function()
                noctalia.openColorPicker(l.mode ~= "white" and l.shown or "#ff8800", function(color)
                    if color == nil then return end
                    act(l, "colour", { "color", color, l.alias }, function(x)
                        x.power, x.mode, x.rgb, x.shown, x.flowing, x.scene = true, "color", string.lower(color), string.lower(color), false, nil
                        x.describe = `On · {x.bright}% · {string.lower(color)}`
                    end)
                end)
            end,
        }),
        ui.label({ text = l.mode ~= "white" and not l.flowing and string.upper(l.shown) or "", color = "on_surface_variant" }),
    }))
    return ui.column({ key = `colour-{l.alias}`, gap = 10, align = "stretch" }, rows)
end

local function whitePage(l)
    local ct = math.clamp(l.ct or 4000, l.ct_min, l.ct_max)
    -- presets as small tiles (icon over name), like the scene tiles: five buttons with text do not fit
    local presets = {}
    for _, w in ipairs(g.WHITES) do
        local k = math.clamp(w.ct, l.ct_min, l.ct_max)
        local current = l.mode == "white" and not l.flowing and l.bright == w.bright and l.ct == k
        table.insert(presets, ui.column({
            key = `wp-{l.alias}-{w.name}`,
            gap = 3, paddingV = 6, paddingH = 2, radius = 10, flexGrow = 1, align = "center",
            fill = current and "primary/0.35" or "surface",
            border = current and "primary" or "outline/0.3", borderWidth = 1,
            tooltip = `{w.bright} %, {k} K`,
            onClick = function()
                act(l, "white", { "white", w.bright, k, l.alias }, function(x)
                    x.power, x.mode, x.bright, x.ct, x.flowing, x.scene = true, "white", w.bright, k, false, nil
                    x.describe = `On · {w.bright}% · {w.name}`
                end)
            end,
        }, {
            ui.glyph({ name = w.glyph, size = 16 }),
            ui.label({ text = w.name, fontSize = 11, maxLines = 1 }),
        }))
    end
    return ui.column({ key = `white-{l.alias}`, gap = 10, align = "stretch" }, {
        ui.row({ key = `ct-{l.alias}`, gap = 8, align = "center", paddingH = 2 }, {
            ui.glyph({ name = "temperature-sun", size = 16, color = "#ff9b3d" }),
            ui.slider({
                key = `cts-{l.alias}`, min = l.ct_min, max = l.ct_max, step = 50, value = ct, width = 180, flexGrow = 1,
                onChange = function(v)
                    local k = math.floor(tonumber(v) or ct)
                    if k == l.ct and l.mode == "white" then return end
                    act(l, "white", { "temp", k, l.alias }, function(x)
                        x.power, x.mode, x.ct, x.flowing, x.scene = true, "white", k, false, nil
                        x.describe = `On · {x.bright}% · {k} K`
                    end)
                end,
            }),
            ui.label({ text = `{ct} K`, width = 60, textAlign = "end", color = "on_surface_variant" }),
        }),
        ui.row({ key = `wps-{l.alias}`, gap = 6, align = "stretch" }, presets),
    })
end

local function scenesPage(l)
    local rows = {}
    for r = 0, 1 do
        local cells = {}
        for c = 1, 5 do
            local s = g.SCENES[r * 5 + c]
            local running = l.flowing and l.scene == s.name
            local strip = {}
            for i, col in ipairs(s.colors) do
                table.insert(strip, ui.box({ key = `sc-{l.alias}-{s.name}-{i}`, height = 12, flexGrow = 1, fill = col }))
            end
            table.insert(cells, ui.column({
                key = `scene-{l.alias}-{s.name}`,
                gap = 5, padding = 6, radius = 10, flexGrow = 1, align = "stretch",
                fill = running and "primary/0.35" or "surface",
                border = running and "primary" or "outline/0.3", borderWidth = 1,
                onClick = function()
                    act(l, "scene", { "scene", string.lower(s.name), l.alias }, function(x)
                        x.power, x.flowing, x.scene = true, true, s.name
                        x.describe = `On · {x.bright}% · {s.name}`
                    end)
                end,
            }, {
                ui.row({ key = `scs-{l.alias}-{s.name}`, gap = 0, radius = 4, align = "stretch" }, strip),
                ui.label({ text = s.name, fontSize = 11, textAlign = "center" }),
            }))
        end
        table.insert(rows, ui.row({ key = `scr-{l.alias}-{r}`, gap = 6, align = "stretch" }, cells))
    end
    if l.flowing then
        table.insert(rows, ui.row({ key = `stop-{l.alias}`, align = "center" }, {
            ui.button({
                key = `stopb-{l.alias}`, glyph = "player-stop", text = "Stop the scene", variant = "secondary", controlSize = "sm",
                onClick = function()
                    act(l, "scene", { "scene", "stop", l.alias }, function(x)
                        x.flowing, x.scene = false, nil
                    end)
                end,
            }),
        }))
    end
    return ui.column({ key = `scenes-{l.alias}`, gap = 6, align = "stretch" }, rows)
end

local function card(l)
    local items = {}
    local isOpenCard = openAlias == l.alias and l.online
    -- head: dot, name + what it does, chevron, switch
    table.insert(items, ui.row({ key = `head-{l.alias}`, gap = 10, align = "center" }, {
        dot(l),
        ui.column({ key = `names-{l.alias}`, gap = 0, flexGrow = 1, align = "start" }, {
            ui.label({ text = l.name, fontWeight = "bold", maxLines = 1 }),
            ui.label({ text = l.describe, color = "on_surface_variant", fontSize = 12, maxLines = 1 }),
        }),
        ui.button({
            key = `chev-{l.alias}`, glyph = "chevron-down", variant = "ghost", controlSize = "sm",
            enabled = l.online,
            visible = not isOpenCard,
            tooltip = "Colour, white and scenes",
            onClick = function()
                openAlias = l.alias
                render()
            end,
        }),
        ui.toggle({
            key = `pow-{l.alias}`, checked = l.online and l.power, enabled = l.online,
            onChange = function(v)
                local on = v == true or v == "true"
                act(l, "power", { on and "on" or "off", l.alias }, function(x)
                    x.power = on
                    x.describe = on and "On" or "Off"
                end)
            end,
        }),
    }))
    -- brightness
    table.insert(items, ui.row({ key = `br-{l.alias}`, gap = 8, align = "center", paddingH = 2 }, {
        ui.glyph({ name = "brightness-down", size = 16, color = "on_surface_variant" }),
        ui.slider({
            key = `brs-{l.alias}`, min = 1, max = 100, step = 1, value = l.bright or 50, width = 180, flexGrow = 1, enabled = l.online,
            onChange = function(v)
                local b = math.floor(tonumber(v) or l.bright)
                if b == l.bright and l.power then return end
                act(l, "bright", { "bright", b, l.alias }, function(x)
                    x.bright, x.power = b, true
                end)
            end,
        }),
        ui.label({ text = `{l.bright or 0} %`, width = 60, textAlign = "end", color = "on_surface_variant" }),
    }))
    if not l.online then
        table.insert(items, ui.row({ key = `off-{l.alias}`, gap = 8, align = "center" }, {
            ui.button({
                key = `retry-{l.alias}`, glyph = "refresh", text = searching and "Looking…" or "Find it again",
                variant = "secondary", controlSize = "sm", enabled = not searching,
                onClick = function()
                    searching = true
                    render()
                    noctalia.runAsync({ g.CMD, "--json", "search" }, function(result)
                        searching = false
                        local ok, rows = pcall(noctalia.json.decode, result.stdout or "")
                        if ok and type(rows) == "table" then g.publish(rows) end
                        render()
                    end)
                end,
            }),
        }))
    elseif isOpenCard then
        local t = tabFor(l)
        if l.can_color or l.can_scenes then
            local tabs = {}
            local function tab(id, text, glyph)
                table.insert(tabs, { text = text, glyph = glyph, selected = t == id, onClick = function()
                    tabOf[l.alias] = id
                    render()
                end })
            end
            if l.can_color then tab("colour", "Colour", "palette") end
            tab("white", "White", "sun")
            if l.can_scenes then tab("scenes", "Scenes", "sparkles") end
            table.insert(items, segmented(`tabs-{l.alias}`, tabs))
        end
        if t == "colour" then
            table.insert(items, colourPage(l))
        elseif t == "scenes" then
            table.insert(items, scenesPage(l))
        else
            table.insert(items, whitePage(l))
        end
    end
    local opts = { key = `card-{l.alias}` }
    for k, v in pairs(CARD) do opts[k] = v end
    return ui.column(opts, items)
end

render = function()
    local rows = {}
    -- header: title, all on, all off
    table.insert(rows, ui.row({ key = "header", gap = 6, align = "center", minHeight = H.header }, {
        ui.label({ text = "Lights", fontWeight = "bold", fontSize = 16, flexGrow = 1 }),
        ui.button({
            key = "all-on", glyph = "bulb", text = "All on", variant = "ghost", controlSize = "sm",
            onClick = function()
                for _, l in ipairs(lights) do
                    if l.online then l.power = true; l.describe = "On" end
                end
                render()
                g.send("power", { "on" })
            end,
        }),
        ui.button({
            key = "all-off", glyph = "bulb-off", text = "All off", variant = "ghost", controlSize = "sm",
            onClick = function()
                for _, l in ipairs(lights) do
                    if l.online then l.power = false; l.describe = "Off" end
                end
                render()
                g.send("power", { "off" })
            end,
        }),
    }))
    if #lights == 0 then
        table.insert(rows, ui.label({ key = "empty", text = "Asking the lights…", color = "on_surface_variant" }))
    end
    for _, l in ipairs(lights) do
        table.insert(rows, card(l))
    end
    setHeight()
    local content = ui.column({ key = "content", gap = H.gap, padding = H.pad, align = "stretch" }, rows)
    if height() > box.room() then
        -- explicit height: Noctalia's scroll view keeps its old viewport when a dropdown grows
        panel.render(ui.scroll({ key = "scroll", height = (box.height() or box.room()) - H.host }, { content }))
    else
        panel.render(content)
    end
end

function onFrameTick(dt)
    if not box.tick(dt) then panel.setNeedsFrameTick(false) end
    render()
end

local function apply(v)
    if type(v) ~= "table" then return end
    lights = v
    if (openAlias == nil or find(openAlias) == nil) and #lights > 0 then
        openAlias = lights[1].alias
    end
    if isOpen then render() else setHeight() end
end

function onOpen(context)
    isOpen = true
    tabOf = {}        -- each open starts on the tab that matches what the light does
    -- `noctalia msg panel-open alecsh/lumen:menu strip` or `... strip:scenes` opens that card (and tab)
    if type(context) == "string" then
        local alias, tab = string.match(context, "^([%w_-]+):?(%a*)$")
        if alias and find(alias) ~= nil then
            openAlias = alias
            if tab == "colour" or tab == "white" or tab == "scenes" then
                tabOf = { [alias] = tab }
            end
        end
    end
    render()
    g.refresh()
end

function onClose()
    isOpen = false
    setHeight()
end

-- while open, read the lights every 3 s (changes from Mi Home or a wall switch)
function update()
    if isOpen then g.refresh() end
end

noctalia.state.watch(g.STATE_KEY, apply)
noctalia.setUpdateInterval(3000)
apply(noctalia.state.get(g.STATE_KEY))
setHeight()
