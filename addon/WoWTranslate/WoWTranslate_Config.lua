-- WoWTranslate_Config.lua
-- Configuration UI panel for WoWTranslate
-- v0.13: Removed API key/credits UI; added source language checkboxes

-- ============================================================================
-- LANGUAGES
-- ============================================================================
local LANGUAGES = {
    { code = "zh", name = "Chinese" },
    { code = "en", name = "English" },
    { code = "ko", name = "Korean" },
    { code = "ja", name = "Japanese" },
    { code = "ru", name = "Russian" },
    { code = "de", name = "German" },
    { code = "fr", name = "French" },
    { code = "es", name = "Spanish" },
    { code = "pt", name = "Portuguese" },
}

local function GetLanguageIndex(code)
    for i = 1, table.getn(LANGUAGES) do
        if LANGUAGES[i].code == code then
            return i
        end
    end
    return 1
end

local function GetLanguageName(code)
    for i = 1, table.getn(LANGUAGES) do
        if LANGUAGES[i].code == code then
            return LANGUAGES[i].name
        end
    end
    return code
end

-- ============================================================================
-- TEMP CONFIG
-- ============================================================================
WoWTranslate_TempConfig = {}

local function LoadTempConfig()
    WoWTranslate_TempConfig = {}
    if not WoWTranslateDB then return end
    for k, v in pairs(WoWTranslateDB) do
        if type(v) == "table" then
            WoWTranslate_TempConfig[k] = {}
            for k2, v2 in pairs(v) do
                WoWTranslate_TempConfig[k][k2] = v2
            end
        else
            WoWTranslate_TempConfig[k] = v
        end
    end
end

local function SaveTempConfig()
    if not WoWTranslate_TempConfig then return end
    for k, v in pairs(WoWTranslate_TempConfig) do
        if type(v) == "table" then
            if not WoWTranslateDB[k] then
                WoWTranslateDB[k] = {}
            end
            for k2, v2 in pairs(v) do
                WoWTranslateDB[k][k2] = v2
            end
        else
            WoWTranslateDB[k] = v
        end
    end
end

-- ============================================================================
-- CREATE MAIN FRAME
-- ============================================================================
local configFrame = CreateFrame("Frame", "WoWTranslateConfigFrame", UIParent)
configFrame:Hide()
configFrame:SetWidth(580)
configFrame:SetHeight(800)
configFrame:SetPoint("CENTER", 0, 0)
configFrame:SetMovable(true)
configFrame:EnableMouse(true)
configFrame:SetClampedToScreen(true)
configFrame:SetFrameStrata("DIALOG")

configFrame:SetBackdrop({
    bgFile = "Interface\\DialogFrame\\UI-DialogBox-Background",
    edgeFile = "Interface\\DialogFrame\\UI-DialogBox-Border",
    tile = true,
    tileSize = 32,
    edgeSize = 32,
    insets = { left = 11, right = 12, top = 12, bottom = 11 }
})
configFrame:SetBackdropColor(0, 0, 0, 1)

configFrame:SetScript("OnMouseDown", function()
    this:StartMoving()
end)

configFrame:SetScript("OnMouseUp", function()
    this:StopMovingOrSizing()
end)

-- Title
local title = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalLarge")
title:SetPoint("TOP", configFrame, "TOP", 0, -20)
title:SetText("WoWTranslate Configuration - v1.5")

-- Close button
local closeBtn = CreateFrame("Button", nil, configFrame, "UIPanelCloseButton")
closeBtn:SetPoint("TOPRIGHT", configFrame, "TOPRIGHT", -5, -5)
closeBtn:SetScript("OnClick", function()
    configFrame:Hide()
end)

-- ESC to close
tinsert(UISpecialFrames, "WoWTranslateConfigFrame")

-- ============================================================================
-- UI ELEMENTS STORAGE
-- ============================================================================
configFrame.elements = {}

-- ============================================================================
-- HELPER: Create Section Header
-- ============================================================================
local function CreateHeader(text, yPos)
    local header = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalLarge")
    header:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, yPos)
    header:SetText(text)
    header:SetTextColor(0, 1, 1)
    return header
end

-- ============================================================================
-- HELPER: Create Checkbox at specific position
-- ============================================================================
local function CreateCheckbox(label, xPos, yPos, configKey, subKey)
    -- Create a wrapper frame like the language selector does
    local wrapper = CreateFrame("Frame", nil, configFrame)
    wrapper:SetPoint("TOPLEFT", configFrame, "TOPLEFT", xPos, yPos)
    wrapper:SetWidth(200)
    wrapper:SetHeight(22)

    -- Store config on wrapper (same pattern as language selector)
    wrapper.configKey = configKey
    wrapper.subKey = subKey

    local cb = CreateFrame("CheckButton", nil, wrapper, "UICheckButtonTemplate")
    cb:SetPoint("TOPLEFT", 0, 0)

    local text = wrapper:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
    text:SetPoint("LEFT", cb, "RIGHT", 4, 0)
    text:SetText(label)

    cb:SetScript("OnClick", function()
        -- Use GetParent() like language selector does
        local parent = this:GetParent()
        local key = parent.configKey
        local sub = parent.subKey

        -- GetChecked() returns 1 or nil in WoW 1.12
        local isChecked = this:GetChecked()
        local enabled = (isChecked and true) or false

        -- Use the global toggle functions for immediate effect
        if key == "translateNameplates" then
            WoWTranslate_SetTranslateNameplates(enabled)
            WoWTranslate_TempConfig.translateNameplates = enabled
        elseif key == "translatePlayerNames" then
            WoWTranslate_SetTranslatePlayerNames(enabled)
            WoWTranslate_TempConfig.translatePlayerNames = enabled
        elseif key == "translateGuildNames" then
            WoWTranslate_SetTranslateGuildNames(enabled)
            WoWTranslate_TempConfig.translateGuildNames = enabled
        elseif key == "translateGroupFinder" then
            WoWTranslate_SetTranslateGroupFinder(enabled)
            WoWTranslate_TempConfig.translateGroupFinder = enabled
        elseif key == "showOutgoingButton" then
            WoWTranslate_SetOutgoingButtonVisible(enabled)
            WoWTranslate_TempConfig.showOutgoingButton = enabled
        elseif key == "outgoingEnabled" then
            WoWTranslate_SetOutgoingEnabled(enabled)
            WoWTranslate_TempConfig.outgoingEnabled = enabled
        elseif key == "enabled" then
            WoWTranslate_SetIncomingEnabled(enabled)
            WoWTranslate_TempConfig.enabled = enabled
        elseif key == "outgoingChannels" and sub then
            WoWTranslate_SetChannelEnabled(sub, enabled)
            if not WoWTranslate_TempConfig.outgoingChannels then
                WoWTranslate_TempConfig.outgoingChannels = {}
            end
            WoWTranslate_TempConfig.outgoingChannels[sub] = enabled
        elseif key == "incomingChannels" and sub then
            WoWTranslate_SetIncomingChannelEnabled(sub, enabled)
            if not WoWTranslate_TempConfig.incomingChannels then
                WoWTranslate_TempConfig.incomingChannels = {}
            end
            WoWTranslate_TempConfig.incomingChannels[sub] = enabled
        else
            -- Fallback for any other settings
            if sub then
                if not WoWTranslate_TempConfig[key] then
                    WoWTranslate_TempConfig[key] = {}
                end
                WoWTranslate_TempConfig[key][sub] = enabled
                if not WoWTranslateDB[key] then
                    WoWTranslateDB[key] = {}
                end
                WoWTranslateDB[key][sub] = enabled
            else
                WoWTranslate_TempConfig[key] = enabled
                WoWTranslateDB[key] = enabled
            end
        end
    end)

    -- Return the checkbox (not wrapper) so SetChecked works
    cb.wrapper = wrapper
    return cb
end

-- ============================================================================
-- HELPER: Create Language Selector
-- ============================================================================
local function CreateLangSelector(label, xPos, yPos, configKey)
    local frame = CreateFrame("Frame", nil, configFrame)
    frame:SetPoint("TOPLEFT", configFrame, "TOPLEFT", xPos, yPos)
    frame:SetWidth(170)
    frame:SetHeight(50)

    local lbl = frame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
    lbl:SetPoint("TOPLEFT", 0, 0)
    lbl:SetText(label)

    local leftBtn = CreateFrame("Button", nil, frame)
    leftBtn:SetPoint("TOPLEFT", lbl, "BOTTOMLEFT", 0, -6)
    leftBtn:SetWidth(24)
    leftBtn:SetHeight(24)
    leftBtn:SetNormalTexture("Interface\\Buttons\\UI-SpellbookIcon-PrevPage-Up")
    leftBtn:SetPushedTexture("Interface\\Buttons\\UI-SpellbookIcon-PrevPage-Down")
    leftBtn:SetHighlightTexture("Interface\\Buttons\\UI-Common-MouseHilight", "ADD")

    local display = frame:CreateFontString(nil, "ARTWORK", "GameFontHighlight")
    display:SetPoint("LEFT", leftBtn, "RIGHT", 10, 0)
    display:SetWidth(85)
    display:SetJustifyH("CENTER")
    display:SetText("Language")

    local rightBtn = CreateFrame("Button", nil, frame)
    rightBtn:SetPoint("LEFT", display, "RIGHT", 10, 0)
    rightBtn:SetWidth(24)
    rightBtn:SetHeight(24)
    rightBtn:SetNormalTexture("Interface\\Buttons\\UI-SpellbookIcon-NextPage-Up")
    rightBtn:SetPushedTexture("Interface\\Buttons\\UI-SpellbookIcon-NextPage-Down")
    rightBtn:SetHighlightTexture("Interface\\Buttons\\UI-Common-MouseHilight", "ADD")

    frame.display = display
    frame.configKey = configKey

    leftBtn:SetScript("OnClick", function()
        local parent = this:GetParent()
        local code = WoWTranslate_TempConfig[parent.configKey] or "zh"
        local idx = GetLanguageIndex(code) - 1
        if idx < 1 then idx = table.getn(LANGUAGES) end
        WoWTranslate_TempConfig[parent.configKey] = LANGUAGES[idx].code
        parent.display:SetText(LANGUAGES[idx].name)
    end)

    rightBtn:SetScript("OnClick", function()
        local parent = this:GetParent()
        local code = WoWTranslate_TempConfig[parent.configKey] or "zh"
        local idx = GetLanguageIndex(code) + 1
        if idx > table.getn(LANGUAGES) then idx = 1 end
        WoWTranslate_TempConfig[parent.configKey] = LANGUAGES[idx].code
        parent.display:SetText(LANGUAGES[idx].name)
    end)

    return frame
end

-- ============================================================================
-- BUILD UI
-- ============================================================================

local Y_IN_HEADER    = -50
local Y_IN_ENABLE    = -76
local Y_IN_NAMES     = -101
local Y_IN_LANG      = -130

local Y_SRC_LABEL    = -185
local Y_SRC_ROW      = -208

local Y_IN_CH_LABEL  = -242
local Y_IN_CH_ROW1   = -264
local Y_IN_CH_ROW2   = -289

local Y_OUT_HEADER   = -322
local Y_OUT_ENABLE   = -349
local Y_OUT_LANG     = -378

local Y_CH_LABEL     = -437
local Y_CH_ROW1      = -459
local Y_CH_ROW2      = -484

local Y_COLOR        = -518
local Y_COLOR_FOLLOW = -542

local Y_EXP_HEADER   = -571
local Y_EXP_ROW      = -593

local Y_NAME_HEADER  = -625
local Y_NAME_ROW     = -647

local Y_SP_HEADER    = -679
local Y_SP_ROW1      = -701
local Y_SP_ROW2      = -723

-- Incoming Translation Section
CreateHeader("Incoming Translation (Chat -> You)", Y_IN_HEADER)
configFrame.elements.inEnabled     = CreateCheckbox("Enable Incoming Translation", 25,  Y_IN_ENABLE, "enabled", nil)
configFrame.elements.afkDisable    = CreateCheckbox("Disable while AFK",          270,  Y_IN_ENABLE, "disableWhileAfk", nil)
configFrame.elements.translateSystem = CreateCheckbox("Translate system/emotes",  25,  Y_IN_NAMES,  "translateSystemMessages", nil)
configFrame.elements.inTo          = CreateLangSelector("To:", 25, Y_IN_LANG, "incomingToLang")

local roleInfoText = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
roleInfoText:SetPoint("TOPRIGHT", configFrame, "TOPRIGHT", -20, Y_IN_LANG - 31)
roleInfoText:SetText("T = tank,  N = healer,  D = dps")
roleInfoText:SetTextColor(0.2, 1, 0.2)
roleInfoText:SetFont("Fonts\\FRIZQT__.TTF", 9, "ITALIC")

local otherInfoText = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
otherInfoText:SetPoint("TOPRIGHT", configFrame, "TOPRIGHT", -20, Y_IN_LANG - 51)
otherInfoText:SetText("M, MM , MMM+ = Whisper")
otherInfoText:SetTextColor(1, 0, 1)
otherInfoText:SetFont("Fonts\\FRIZQT__.TTF", 9, "ITALIC")

-- Source Language Selection
local srcLabel = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
srcLabel:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_SRC_LABEL)
srcLabel:SetText("Translate incoming from:")

configFrame.elements.srcZH = CreateCheckbox("Chinese",  25,  Y_SRC_ROW, "enabledSourceLangs", "zh")
configFrame.elements.srcJA = CreateCheckbox("Japanese", 115, Y_SRC_ROW, "enabledSourceLangs", "ja")
configFrame.elements.srcKO = CreateCheckbox("Korean",   210, Y_SRC_ROW, "enabledSourceLangs", "ko")
configFrame.elements.srcRU = CreateCheckbox("Russian",  300, Y_SRC_ROW, "enabledSourceLangs", "ru")
configFrame.elements.srcEN = CreateCheckbox("English",  390, Y_SRC_ROW, "enabledSourceLangs", "en")

-- Incoming Channels Section
local inChLabel = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
inChLabel:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_IN_CH_LABEL)
inChLabel:SetText("Translate Incoming Channels:")

-- Row 1: Say, Yell, Whisper, Party, Guild
configFrame.elements.inChSay     = CreateCheckbox("Say",          25,  Y_IN_CH_ROW1, "incomingChannels", "SAY")
configFrame.elements.inChYell    = CreateCheckbox("Yell",        115,  Y_IN_CH_ROW1, "incomingChannels", "YELL")
configFrame.elements.inChWhisper = CreateCheckbox("Whisper",     205,  Y_IN_CH_ROW1, "incomingChannels", "WHISPER")
configFrame.elements.inChParty   = CreateCheckbox("Party",       310,  Y_IN_CH_ROW1, "incomingChannels", "PARTY")
configFrame.elements.inChGuild   = CreateCheckbox("Guild",       405,  Y_IN_CH_ROW1, "incomingChannels", "GUILD")

-- Row 2: Raid, English, Battleground, World/Local, Hardcore
configFrame.elements.inChRaid    = CreateCheckbox("Raid",         25,  Y_IN_CH_ROW2, "incomingChannels", "RAID")
configFrame.elements.inChEnglish = CreateCheckbox("English",     115,  Y_IN_CH_ROW2, "incomingChannels", "ENGLISH")
configFrame.elements.inChBG      = CreateCheckbox("Battleground", 210, Y_IN_CH_ROW2, "incomingChannels", "BATTLEGROUND")
configFrame.elements.inChChannel = CreateCheckbox("World/Local",  315, Y_IN_CH_ROW2, "incomingChannels", "CHANNEL")
configFrame.elements.inChHC      = CreateCheckbox("Hardcore",     415, Y_IN_CH_ROW2, "incomingChannels", "HARDCORE")

-- Outgoing Translation Section
CreateHeader("Outgoing Translation (You -> Chat)", Y_OUT_HEADER)
configFrame.elements.outEnabled   = CreateCheckbox("Enable Outgoing Translation",  25,  Y_OUT_ENABLE, "outgoingEnabled",       nil)
configFrame.elements.outPrefix    = CreateCheckbox("Send prefix with translation",  210, Y_OUT_ENABLE, "outgoingPrefixEnabled", nil)
configFrame.elements.outShowBtn   = CreateCheckbox("Toggle Button",                 410, Y_OUT_ENABLE, "showOutgoingButton",    nil)
configFrame.elements.outFrom    = CreateLangSelector("From:", 25,  Y_OUT_LANG, "outgoingFromLang")
configFrame.elements.outTo      = CreateLangSelector("To:",  215,  Y_OUT_LANG, "outgoingToLang")

-- Outgoing Channels Section
local chLabel = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
chLabel:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_CH_LABEL)
chLabel:SetText("Outgoing Channels:")

-- Row 1: Whisper, Party, Say, Guild, Raid
configFrame.elements.chWhisper = CreateCheckbox("Whisper",  25,  Y_CH_ROW1, "outgoingChannels", "WHISPER")
configFrame.elements.chParty   = CreateCheckbox("Party",   115,  Y_CH_ROW1, "outgoingChannels", "PARTY")
configFrame.elements.chSay     = CreateCheckbox("Say",     210,  Y_CH_ROW1, "outgoingChannels", "SAY")
configFrame.elements.chGuild   = CreateCheckbox("Guild",   300,  Y_CH_ROW1, "outgoingChannels", "GUILD")
configFrame.elements.chRaid    = CreateCheckbox("Raid",    390,  Y_CH_ROW1, "outgoingChannels", "RAID")

-- Row 2: Yell, English, Battleground, World/Local, Hardcore
configFrame.elements.chYell    = CreateCheckbox("Yell",         25,  Y_CH_ROW2, "outgoingChannels", "YELL")
configFrame.elements.chEnglish = CreateCheckbox("English",     115,  Y_CH_ROW2, "outgoingChannels", "ENGLISH")
configFrame.elements.chBG      = CreateCheckbox("Battleground", 210, Y_CH_ROW2, "outgoingChannels", "BATTLEGROUND")
configFrame.elements.chChannel = CreateCheckbox("World/Local",  315, Y_CH_ROW2, "outgoingChannels", "CHANNEL")
configFrame.elements.chHC      = CreateCheckbox("Hardcore",     415, Y_CH_ROW2, "outgoingChannels", "HARDCORE")

-- Translation Color Section — all controls on one line.
-- Frames MUST anchor to configFrame (not to FontStrings) in WoW 1.12;
-- FontStrings may anchor to Frames freely.
local colorSectionLabel = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
colorSectionLabel:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_COLOR)
colorSectionLabel:SetText("Translation text color:")

local colorSwatch = CreateFrame("Button", "WoWTranslateColorSwatch", configFrame)
colorSwatch:SetWidth(30)
colorSwatch:SetHeight(18)
colorSwatch:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 196, Y_COLOR - 2)
colorSwatch:SetBackdrop({
    bgFile   = "Interface\\Buttons\\WHITE8x8",
    edgeFile = "Interface\\Buttons\\WHITE8x8",
    tile     = true, tileSize = 8, edgeSize  = 1,
    insets   = { left = 1, right = 1, top = 1, bottom = 1 },
})
colorSwatch:SetBackdropBorderColor(0, 0, 0)
colorSwatch:SetBackdropColor(1, 1, 1)

local colorSwatchLabel = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormalSmall")
colorSwatchLabel:SetPoint("LEFT", colorSwatch, "RIGHT", 6, 0)
colorSwatchLabel:SetText("(click to pick)")

local colorDefaultBtn = CreateFrame("Button", nil, configFrame, "UIPanelButtonTemplate")
colorDefaultBtn:SetWidth(70)
colorDefaultBtn:SetHeight(18)
colorDefaultBtn:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 322, Y_COLOR - 2)
colorDefaultBtn:SetText("Default")

local function ApplyTranslationColor(hex)
    WoWTranslate_TempConfig.translationColor = hex
    if WoWTranslateDB then WoWTranslateDB.translationColor = hex end
    if hex and string.len(hex) == 6 then
        local r = tonumber(string.sub(hex, 1, 2), 16) / 255
        local g = tonumber(string.sub(hex, 3, 4), 16) / 255
        local b = tonumber(string.sub(hex, 5, 6), 16) / 255
        colorSwatch:SetBackdropColor(r, g, b)
    else
        colorSwatch:SetBackdropColor(0.5, 0.5, 0.5)
    end
end

colorSwatch:SetScript("OnClick", function()
    local hex = (WoWTranslateDB and WoWTranslateDB.translationColor) or ""
    local r, g, b = 1, 1, 1
    if hex and string.len(hex) == 6 then
        r = tonumber(string.sub(hex, 1, 2), 16) / 255
        g = tonumber(string.sub(hex, 3, 4), 16) / 255
        b = tonumber(string.sub(hex, 5, 6), 16) / 255
    end
    ColorPickerFrame.hasOpacity = false
    ColorPickerFrame.func = function()
        local nr, ng, nb = ColorPickerFrame:GetColorRGB()
        local nhex = string.format("%02X%02X%02X",
            math.floor(nr * 255), math.floor(ng * 255), math.floor(nb * 255))
        ApplyTranslationColor(nhex)
    end
    ColorPickerFrame.cancelFunc = function(pv)
        local pr, pg, pb = pv[1], pv[2], pv[3]
        local phex = string.format("%02X%02X%02X",
            math.floor(pr * 255), math.floor(pg * 255), math.floor(pb * 255))
        ApplyTranslationColor(phex)
    end
    ColorPickerFrame.previousValues = { r, g, b }
    ColorPickerFrame:SetColorRGB(r, g, b)
    ColorPickerFrame:SetFrameStrata("FULLSCREEN_DIALOG")
    ShowUIPanel(ColorPickerFrame)
end)

colorDefaultBtn:SetScript("OnClick", function()
    ApplyTranslationColor("")
end)

configFrame.elements.colorSwatch = colorSwatch

configFrame.elements.colorFollow = CreateCheckbox("Follow channel color", 25, Y_COLOR_FOLLOW, "translationColorFollow", nil)

-- Experimental Section
local expHeader = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormal")
expHeader:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_EXP_HEADER)
expHeader:SetText("Experimental:")
expHeader:SetTextColor(1, 0.5, 0)

configFrame.elements.replaceMode      = CreateCheckbox("Replace original with translation", 25,  Y_EXP_ROW, "replaceMode", nil)
configFrame.elements.translateGF      = CreateCheckbox("Translate Group Finder",           270, Y_EXP_ROW, "translateGroupFinder", nil)

-- Name Translation Section
CreateHeader("Name Translation:", Y_NAME_HEADER)
configFrame.elements.translateNames  = CreateCheckbox("Sender names (chat/tooltip)", 25,  Y_NAME_ROW, "translatePlayerNames", nil)
configFrame.elements.translateGuilds = CreateCheckbox("Guild names (tooltip)",       290, Y_NAME_ROW, "translateGuildNames", nil)

-- ShaguPlates Section
local spHeader = configFrame:CreateFontString(nil, "ARTWORK", "GameFontNormal")
spHeader:SetPoint("TOPLEFT", configFrame, "TOPLEFT", 25, Y_SP_HEADER)
spHeader:SetText("ShaguPlates:")
spHeader:SetTextColor(0, 1, 1)

configFrame.elements.translateNP    = CreateCheckbox("Translate Nameplates",        25,  Y_SP_ROW1, "translateNameplates",   nil)
configFrame.elements.npClassColor   = CreateCheckbox("Class-colored names",         290, Y_SP_ROW1, "playerNameClassColor",  nil)
configFrame.elements.npGuildOOC     = CreateCheckbox("Guild (out of combat)",        25,  Y_SP_ROW2, "nameplateGuildOOC",     nil)
configFrame.elements.npHideHealth   = CreateCheckbox("Hide healthbar (out of combat)", 290, Y_SP_ROW2, "nameplateHideHealthOOC", nil)

-- Bottom Buttons
local clearBtn = CreateFrame("Button", nil, configFrame, "UIPanelButtonTemplate")
clearBtn:SetPoint("BOTTOMLEFT", configFrame, "BOTTOMLEFT", 25, 12)
clearBtn:SetWidth(120)
clearBtn:SetHeight(26)
clearBtn:SetText("Clear Cache")
clearBtn:SetScript("OnClick", function()
    if WoWTranslate_CacheClear then
        WoWTranslate_CacheClear()
        DEFAULT_CHAT_FRAME:AddMessage("|cFFFFFF00[WoWTranslate] Cache cleared|r")
    end
end)

local saveBtn = CreateFrame("Button", nil, configFrame, "UIPanelButtonTemplate")
saveBtn:SetPoint("BOTTOMRIGHT", configFrame, "BOTTOMRIGHT", -25, 12)
saveBtn:SetWidth(80)
saveBtn:SetHeight(26)
saveBtn:SetText("Save")
saveBtn:SetScript("OnClick", function()
    SaveTempConfig()
    DEFAULT_CHAT_FRAME:AddMessage("|cFF00FF00[WoWTranslate] Settings saved!|r")
    configFrame:Hide()
end)

-- ============================================================================
-- REFRESH UI FROM CONFIG
-- ============================================================================
local function RefreshUI()
    local e = configFrame.elements
    local cfg = WoWTranslate_TempConfig

    if e.colorSwatch then
        local hex = cfg.translationColor or ""
        if hex and string.len(hex) == 6 then
            local r = tonumber(string.sub(hex, 1, 2), 16) / 255
            local g = tonumber(string.sub(hex, 3, 4), 16) / 255
            local b = tonumber(string.sub(hex, 5, 6), 16) / 255
            e.colorSwatch:SetBackdropColor(r, g, b)
        else
            e.colorSwatch:SetBackdropColor(0.5, 0.5, 0.5)
        end
    end
    if e.colorFollow then e.colorFollow:SetChecked(cfg.translationColorFollow) end
    if e.replaceMode    then e.replaceMode:SetChecked(cfg.replaceMode) end
    if e.translateGF    then e.translateGF:SetChecked(cfg.translateGroupFinder) end
    if e.translateNames then e.translateNames:SetChecked(cfg.translatePlayerNames) end
    if e.translateGuilds then e.translateGuilds:SetChecked(cfg.translateGuildNames) end
    if e.translateNP    then e.translateNP:SetChecked(cfg.translateNameplates) end
    if e.npClassColor   then e.npClassColor:SetChecked(cfg.playerNameClassColor) end
    if e.npGuildOOC     then e.npGuildOOC:SetChecked(cfg.nameplateGuildOOC) end
    if e.npHideHealth   then e.npHideHealth:SetChecked(cfg.nameplateHideHealthOOC) end
    if e.inEnabled then e.inEnabled:SetChecked(cfg.enabled) end
    if e.afkDisable then e.afkDisable:SetChecked(cfg.disableWhileAfk) end
    if e.translateSystem then e.translateSystem:SetChecked(cfg.translateSystemMessages) end
    if e.outEnabled  then e.outEnabled:SetChecked(cfg.outgoingEnabled) end
    if e.outPrefix   then e.outPrefix:SetChecked(cfg.outgoingPrefixEnabled) end
    if e.outShowBtn  then e.outShowBtn:SetChecked(cfg.showOutgoingButton) end

    -- Source language checkboxes
    local srcLangs = cfg.enabledSourceLangs or {}
    if e.srcZH then e.srcZH:SetChecked(srcLangs.zh) end
    if e.srcJA then e.srcJA:SetChecked(srcLangs.ja) end
    if e.srcKO then e.srcKO:SetChecked(srcLangs.ko) end
    if e.srcRU then e.srcRU:SetChecked(srcLangs.ru) end
    if e.srcEN then e.srcEN:SetChecked(srcLangs.en) end

    if e.inTo and e.inTo.display then
        e.inTo.display:SetText(GetLanguageName(cfg.incomingToLang or "en"))
    end
    if e.outFrom and e.outFrom.display then
        e.outFrom.display:SetText(GetLanguageName(cfg.outgoingFromLang or "en"))
    end
    if e.outTo and e.outTo.display then
        e.outTo.display:SetText(GetLanguageName(cfg.outgoingToLang or "zh"))
    end

    -- Incoming channels
    local inCh = cfg.incomingChannels or {}
    if e.inChSay then e.inChSay:SetChecked(inCh.SAY) end
    if e.inChYell then e.inChYell:SetChecked(inCh.YELL) end
    if e.inChWhisper then e.inChWhisper:SetChecked(inCh.WHISPER) end
    if e.inChParty then e.inChParty:SetChecked(inCh.PARTY) end
    if e.inChGuild then e.inChGuild:SetChecked(inCh.GUILD) end
    if e.inChRaid then e.inChRaid:SetChecked(inCh.RAID) end
    if e.inChBG then e.inChBG:SetChecked(inCh.BATTLEGROUND) end
    if e.inChChannel then e.inChChannel:SetChecked(inCh.CHANNEL) end
    if e.inChHC then e.inChHC:SetChecked(inCh.HARDCORE) end
    if e.inChEnglish then e.inChEnglish:SetChecked(inCh.ENGLISH) end

    -- Outgoing channels
    local ch = cfg.outgoingChannels or {}
    if e.chWhisper then e.chWhisper:SetChecked(ch.WHISPER) end
    if e.chParty then e.chParty:SetChecked(ch.PARTY) end
    if e.chSay then e.chSay:SetChecked(ch.SAY) end
    if e.chGuild then e.chGuild:SetChecked(ch.GUILD) end
    if e.chRaid then e.chRaid:SetChecked(ch.RAID) end
    if e.chYell then e.chYell:SetChecked(ch.YELL) end
    if e.chBG then e.chBG:SetChecked(ch.BATTLEGROUND) end
    if e.chChannel then e.chChannel:SetChecked(ch.CHANNEL) end
    if e.chHC then e.chHC:SetChecked(ch.HARDCORE) end
    if e.chEnglish then e.chEnglish:SetChecked(ch.ENGLISH) end
end

-- ============================================================================
-- PUBLIC API
-- ============================================================================
function WoWTranslate_ShowConfig()
    LoadTempConfig()
    RefreshUI()
    configFrame:Show()
end

function WoWTranslate_HideConfig()
    configFrame:Hide()
end

function WoWTranslate_ToggleConfig()
    if configFrame:IsVisible() then
        configFrame:Hide()
    else
        WoWTranslate_ShowConfig()
    end
end
