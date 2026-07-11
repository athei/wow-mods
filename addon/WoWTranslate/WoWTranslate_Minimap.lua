-- WoWTranslate_Minimap.lua
-- Minimap button for WoWTranslate (Atlas pattern)
-- Left-click toggles config panel, drag to reposition around minimap edge

local MINIMAP_BUTTON_RADIUS = 80
local DEFAULT_POSITION = 225  -- degrees, bottom-left area
local isDragging = false

-- ============================================================================
-- UPDATE POSITION (polar -> cartesian)
-- ============================================================================
local function UpdatePosition()
    if not WoWTranslateMinimapButton then return end
    local angle = DEFAULT_POSITION
    if WoWTranslateDB and WoWTranslateDB.minimapPos then
        angle = tonumber(WoWTranslateDB.minimapPos) or DEFAULT_POSITION
    end
    local rads = math.rad(angle)
    local x = 53 - (MINIMAP_BUTTON_RADIUS * math.cos(rads))
    local y = (MINIMAP_BUTTON_RADIUS * math.sin(rads)) - 55
    WoWTranslateMinimapButton:ClearAllPoints()
    WoWTranslateMinimapButton:SetPoint("TOPLEFT", Minimap, "TOPLEFT", x, y)
end

-- ============================================================================
-- CREATE BUTTON (single Button on Minimap, Atlas pattern)
-- ============================================================================
local button = CreateFrame("Button", "WoWTranslateMinimapButton", Minimap)
button:SetWidth(33)
button:SetHeight(33)
button:SetFrameStrata("MEDIUM")
button:SetFrameLevel(8)
button:EnableMouse(true)
button:SetMovable(true)
button:RegisterForClicks("LeftButtonUp")
button:RegisterForDrag("LeftButton")

-- Icon texture (scroll/note — fits "translation" theme)
local icon = button:CreateTexture(nil, "ARTWORK")
icon:SetTexture("Interface\\Icons\\INV_Misc_Note_01")
icon:SetWidth(20)
icon:SetHeight(20)
icon:SetPoint("CENTER", button, "CENTER", 0, 0)

-- Border texture (standard minimap button border)
local border = button:CreateTexture(nil, "OVERLAY")
border:SetTexture("Interface\\Minimap\\MiniMap-TrackingBorder")
border:SetWidth(52)
border:SetHeight(52)
border:SetPoint("TOPLEFT", button, "TOPLEFT", 0, 0)

-- Highlight texture
local highlight = button:CreateTexture(nil, "HIGHLIGHT")
highlight:SetTexture("Interface\\Minimap\\UI-Minimap-ZoomButton-Highlight")
highlight:SetWidth(24)
highlight:SetHeight(24)
highlight:SetPoint("CENTER", button, "CENTER", 0, 0)
highlight:SetBlendMode("ADD")

-- ============================================================================
-- DRAG LOGIC
-- ============================================================================
button:SetScript("OnDragStart", function()
    isDragging = true
    this:SetScript("OnUpdate", function()
        local mx, my = Minimap:GetCenter()
        local scale = Minimap:GetScale()
        local cx, cy = GetCursorPosition()
        local uiScale = UIParent:GetScale()
        cx = cx / (scale * uiScale)
        cy = cy / (scale * uiScale)
        mx = mx / uiScale
        my = my / uiScale
        local angle = math.deg(math.atan2(cy - my, cx - mx))
        if not WoWTranslateDB then WoWTranslateDB = {} end
        WoWTranslateDB.minimapPos = angle
        UpdatePosition()
    end)
end)

button:SetScript("OnDragStop", function()
    isDragging = false
    this:SetScript("OnUpdate", nil)
end)

-- ============================================================================
-- CLICK HANDLER
-- ============================================================================
button:SetScript("OnClick", function()
    if isDragging then return end
    if WoWTranslate_ToggleConfig then
        WoWTranslate_ToggleConfig()
    end
end)

-- ============================================================================
-- TOOLTIP
-- ============================================================================
button:SetScript("OnEnter", function()
    if isDragging then return end
    GameTooltip:SetOwner(this, "ANCHOR_LEFT")
    GameTooltip:AddLine("WoWTranslate")
    GameTooltip:AddLine("Click to open settings", 0.8, 0.8, 0.8)
    GameTooltip:Show()
end)

button:SetScript("OnLeave", function()
    GameTooltip:Hide()
end)

-- ============================================================================
-- INITIALIZATION (called from WoWTranslate.lua after settings are loaded)
-- ============================================================================
function WoWTranslate_MinimapButton_Init()
    if not WoWTranslateDB then WoWTranslateDB = {} end
    if WoWTranslateDB.minimapPos == nil then
        WoWTranslateDB.minimapPos = DEFAULT_POSITION
    end
    UpdatePosition()
end
