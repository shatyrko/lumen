--!nonstrict
-- fit.luau: a plugin dropdown that is as tall as its content, up to the screen, and only then scrolls;
-- height changes glide (one panel.setHeight per frame). Same method as the Git Updater's and the Arch
-- Updater's dropdowns. Needs the kit's Noctalia patches plugin-panel-height + plugin-panel-live-height;
-- without them panel.setHeight is missing and the panel keeps plugin.toml's height.
-- One copy per plugin (arch-setup kit, 2026-10-25): system-stats, world-clock, lumen.
--
--   local fit = require("./fit.luau")
--   local box = fit.new({ min = 160 })
--   box.fit(h)            the content wants h px (panel padding included); first call jumps, later ones glide
--   box.room()            the most the panel may take: the screen under the bar
--   box.height()          the height the panel has now (nil before the first fit)
--   box.tick(dt)          from onFrameTick; true while still gliding
--   box.reset()           on open: the next fit jumps instead of gliding
--   fit.measure(node, w)  estimated height of a ui.* tree laid out w px wide

local M = {}

local function ease(t) return 1 - (1 - t) ^ 3 end

function M.new(opts)
    opts = opts or {}
    local MIN = opts.min or 120
    local MS = opts.ms or 230
    local MARGIN = opts.margin or 110       -- the bar, the gap under it and a little air at the bottom
    local cur, grow = nil, nil
    local self = {}

    function self.room()
        local screenH = nil
        for _, o in ipairs(noctalia.outputs() or {}) do
            if o.focused or screenH == nil then screenH = o.height end
        end
        return math.max(MIN, (screenH or 1080) - MARGIN)
    end

    function self.height() return cur end

    function self.fit(h)
        if panel.setHeight == nil then return end
        h = math.clamp(math.ceil(h), MIN, self.room())
        if grow ~= nil then
            if grow.to == h then return end
            grow = { from = cur, to = h, t = 0 }        -- a new goal while gliding: from where it is now
            return
        end
        if cur == nil then
            cur = h
            panel.setHeight(h)
        elseif h ~= cur then
            grow = { from = cur, to = h, t = 0 }
            panel.setNeedsFrameTick(true)
        end
    end

    function self.tick(dt)
        if grow == nil then return false end
        grow.t = math.min(1, grow.t + (tonumber(dt) or 16) / MS)
        cur = math.floor(grow.from + (grow.to - grow.from) * ease(grow.t) + 0.5)
        panel.setHeight(cur)
        if grow.t >= 1 then
            cur = grow.to
            grow = nil
            return false
        end
        return true
    end

    function self.gliding() return grow ~= nil end

    function self.reset()
        grow = nil
        cur = nil
    end

    return self
end

-- Estimated height of a ui.* tree. Explicit `height` wins; unknown controls count as 0 (a short guess
-- only means a scroll bar, never cut-off content, as long as the body sits in a scroll view).
function M.measure(node, width)
    if type(node) ~= "table" then return 0 end
    local p = node.props or {}
    local kids = node.children or {}
    if p.visible == false then return 0 end
    if p.height then return p.height end
    local t = node.type
    local padV = (p.paddingV or p.padding or 0) * 2 + (p.paddingTop or 0) + (p.paddingBottom or 0)
    local padH = 2 * (p.paddingH or p.padding or 0)
    local h = 0
    if t == "column" or t == "scroll" then
        local sum, n = 0, 0
        for _, c in ipairs(kids) do
            local ch = M.measure(c, width - padH)
            if ch > 0 then sum += ch; n += 1 end
        end
        h = sum + math.max(0, n - 1) * (p.gap or 0) + padV
    elseif t == "row" then
        local m = 0
        for _, c in ipairs(kids) do m = math.max(m, M.measure(c, width - padH)) end
        h = m + padV
    elseif t == "label" then
        local fs = p.fontSize or 14
        local line = math.ceil(fs * 1.45)
        local maxLines = p.maxLines or 1
        if maxLines > 1 and type(p.text) == "string" then
            local perLine = math.max(10, math.floor((p.maxWidth or width) / (fs * 0.55)))
            h = line * math.min(maxLines, math.max(1, math.ceil(#p.text / perLine)))
        else
            h = line
        end
    elseif t == "button" then
        h = (p.controlSize == "sm" and 32) or (p.controlSize == "lg" and 44) or 38
    elseif t == "glyph" then
        h = p.size or 16
    elseif t == "separator" then
        h = 1 + 2 * (p.spacing or 4)
    elseif t == "slider" then
        h = 28
    elseif t == "toggle" then
        h = 24
    elseif t == "input" or t == "select" then
        h = 36
    end
    return math.max(h, p.minHeight or 0)
end

return M
