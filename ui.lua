---@diagnostic disable: undefined-global
glfw = require("glfw")

local RenderCommand = {
	ShapeColor = function(shape, color)
		return {
			"shapeColor",
			shape = shape,
			color = color
		}
	end
}

local function componentSetup(props)
	props.bounds = function(self)
		local w, h = self.size.w or 0.0, self.size.h or 0.0
		local x, y = self.pos.x or 0.0, self.pos.y or 0.0

		local pref = self.prefSize
		if pref then
			w = pref.w or w
			h = pref.h or h
		end
		local pref = self.prefPos
		if pref then
			x = pref.x or x
			y = pref.y or y
		end

		local padding = self.padding
		if padding then
			local left = padding.left or 0.0
			local right = padding.right or 0.0
			local bottom = padding.bottom or 0.0
			local top = padding.top or 0.0
			w = w - left - right;
			h = h - top - bottom;
			x = x + left;
			y = y + bottom;
		end

		return x, y, w, h
	end
end

local function ternary(cond, a, b)
	if cond then
		return a
	else
		return b
	end
end

local Component = {
	Root = function(props)
		componentSetup(props)

		props.render = function(self)
			local x, y, w, h = self:bounds()
			for _, child in ipairs(self.children) do
				child.size = { w = w, h = h };
				child.pos = { x = x, y = y };
				child.parent = self;
				if child.render then
					child:render()
				end
			end
		end
		props.getChildrenList = function(self)
			return self.children
		end
		return props
	end,

	Group = function(props)
		componentSetup(props)
		props.render = function(self)
			local flow = self.flow
			local fx = flow == "right"
			local fy = flow == "up"
			local x, y, w, h = self:bounds()
			local sum = 0.0
			for _, child in ipairs(self.children) do
				local wi, _ = table.unpack(child)
				sum = sum + wi
			end
			for _, child in ipairs(self.children) do
				local wi, child = table.unpack(child)
				wi = wi / sum
				local cw = ternary(fx, w * wi, nil)
				local dx = cw or 0.0
				local ch = ternary(fy, h * wi, nil)
				local dy = ch or 0.0
				if child then
					print(fx, fy, cw, dx, ch, dy)
					child.size = { w = cw or w, h = ch or h };
					child.pos = { x = x, y = y };
					child.parent = self;
					if child.render then
						child:render()
					end
				end
				x = x + dx;
				y = y + dy;
			end
		end
		props.getChildrenList = function(self)
			local children = {}
			for i, v in pairs(self.children) do
				local _, child = table.unpack(v)
				if child then
					table.insert(children, child)
				end
			end
			return children
		end
		return props
	end,

	Slider = function(props)
		componentSetup(props)
		props.render = function(self)
			local x, y, w, h = self:bounds(self)
			local p = self.progress or 0.0
			ui:push {
				"shapeColor",
				shape = { "rect", x, y, w, h },
				color = self.background
			}

			self.cmd = {
				"shapeColor",
				shape = { "rect", x, y, w * p, h },
				color = self.foreground
			}
			ui:push {
				"indirect",
				command = self.cmd
			}
		end
		props.setPosition = function(self, p)
			self.progress = p
			local p = self.progress or 0.0
			local x, y, w, h = self:bounds(self)
			self.cmd.shape = { "rect", x, y, w * p, h };
			self.cmd.color = self.foreground;
			self.cmd.dirty = true
		end
		return props
	end,

	VideoPlayer = function(path)
		local surface = ui:newVideoSurface()
		local player = ui:newVideoPlayer(path, surface)
		local props = {
			surface = surface,
			player = player,
			render = function(self)
				local x, y, w, h = self:bounds(self)
				self.cmd = {
					"videoSurface",
					shape = { "rect", x, y, w, h },
					surface = self.surface
				}
				ui:push {
					"indirect",
					command = self.cmd
				}
			end,
			play = function(self)
				return self.player.player:play()
			end,
		}
		componentSetup(props)
		return props
	end,

	Label = function(props)
		componentSetup(props)
		props.render = function(self)
            self.text = ui:newText(self.str or "", self.font or "default", self.fontSize or 16.0)
			local x, y, w, h = self:bounds(self)
			local wx = {
			    right = 1.0,
			    center = 0.5,
			    left = 0.0
			}
			local wy = {
			    top = 1.0,
			    center = 0.5,
			    bottom = 0.0
			}
			ui:fitText(self.text, w, h)
			local tx, ty, tw, th = self.text:bounds()
			self.text.x = x + (w - tw) * wx[self.alignHorizontal or "center"]
			self.text.y = y + (h - th) * wy[self.alignVertical or "center"]
			ui:push {
				"textFill",
				text = self.text,
				color = self.color
			}
		end
        props.setText = function(self, text)
            ui:setText(self.text, text)
            self.str = text
        end

		return props
	end,

	Rect = function(props)
		componentSetup(props)
		props.render = function(self)
			local x, y, w, h = self:bounds(self)
			ui:push {
				"shapeColor",
				shape = { "rect", x, y, w, h },
				color = self.color
			}
		end

		return props
	end,
}


function dump(o)
	if type(o) == 'table' then
		local s = '{ '
		for k,v in pairs(o) do
			local k = k
			if type(k) ~= 'number' then k = '"'..k..'"' end
			s = s .. '['..k..'] = ' .. dump(v) .. ','
		end
		return s .. '} '
	else
		return tostring(o)
	end
end

local eventHandlers = {
	key = function(self, e)
		if onKey then
			onKey(e)
		end
		if self.focused and self.focused.onKey then
			self.focused:onKey(e)
		end
	end,
	mouseMoved = function(self, e)
		if onMouseMoved then
			onMouseMoved(e)
		end
		local e = e
		local x, y = e.to.x, e.to.y
		if bodies then
			for _, v in pairs(bodies) do
				if not e then
					break
				end
				if v.component.onMouseMoved then
					local cx, cy, w, h = table.unpack(v.bounds)
					if x >= cx and x <= cx + w and y >= cy and y <= cy + h then
						e = v.component:onMouseMoved(e)
					end
				end
			end
		end
	end,
	mouseButton = function(self, e)
		if onMouseButton then
			onMouseButton(e)
		end
		local e = e
		local x, y = e.pos.x, e.pos.y
		if bodies then
			for _, v in pairs(bodies) do
				if not e then
					break
				end
				if v.component.onMouseButton then
					local cx, cy, w, h = table.unpack(v.bounds)
					if x >= cx and x <= cx + w and y >= cy and y <= cy + h then
						e = v.component:onMouseButton(e)
					end
				end
			end
		end
	end
}

function event(e)
	local handler = eventHandlers[e.type]
	if handler then
		handler(eventHandlers, e)
	end
end

function format_time(seconds)
    local seconds = seconds
    local hours = math.floor(seconds / 3600.0)
    seconds = seconds - hours * 3600.0
    local minutes = math.floor(seconds / 60.0)
    seconds = seconds - minutes * 60.0
    seconds = math.floor(seconds)
    return string.format("%02d:%02d:%02d", hours, minutes, seconds)
end

function update()
	local pl = player.player
	local player = pl.player
	if player and timeline then
		local last = pl.lastPts or 0.0
		local pts = player.pts
		local diff = pts - last
		if diff < 0.0 then
			diff = -diff
		end
		if diff >= 0.1 then
			timeline:setPosition(pts / player.duration)
			pl.lastPts = pts
			time_passed:setText(format_time(pts))
			time_rem:setText(format_time(player.duration - pts))
		end
	end
end

player = Component.VideoPlayer("tt.webm")
player:play()

timeline = Component.Slider {
	background = { 0.2, 0.4, 0.5, 1.0 },
	foreground = { 0.2, 1.0, 1.0, 1.0 },
	onMouseButton = function(self, e)
		if e.action ~= glfw.action.release then
			return
		end
		local mx = e.pos.x
		local x, _, w, _ = self:bounds()
		local player = player.player.player
		player:seek(player.duration * ((mx - x) / w))
	end
}

time_passed = Component.Label {
    alignHorizontal = "left",
    color = { 1.0, 1.0, 1.0, 1.0 },
    str = "00:00:00"
}
time_rem = Component.Label {
    alignHorizontal = "right",
    color = { 1.0, 1.0, 1.0, 1.0 },
    str = "00:00:00"
}

root = Component.Root {
	children = {
		player,
		Component.Root {
			children = {
				Component.Rect {
					color = { 0.7, 0.3, 1.0, 1.0 },
					padding = {
						top = 15.0, right = 15.0,
						left = 15.0, bottom = 15.0
					},
				},
				Component.Group {
					flow = "up",
					children = {
						{ 2.0, Component.Group {
						    flow = "right",
						    children = {
						        { 1.0, time_passed},
                                { 3.0, nil },
						        { 1.0, time_rem}
						    }
						}},
						{ 1.0, timeline },
						{ 2.0, nil },
					},
					padding = {
						top = 15.0, right = 15.0,
						left = 15.0, bottom = 15.0
					},
				},
			},
			prefSize = { h = 200.0 }
		},
	},
	onMouseButton = function(self, e)
		print("Root... I don't care about you")
	end
}

function getBodies(curr)
	local x, y, w, h = curr:bounds()
	local bodies = {
		{ bounds = { x, y, w, h }, component = curr }
	}
	if curr.getChildrenList then
		for _, v in pairs(curr:getChildrenList()) do
			for _, v in pairs(getBodies(v)) do
				table.insert(bodies, 1, v)
			end
		end
	end
	return bodies
end

function render()
	if root and root.render then
		root.pos = { x = 0.0, y = 0.0 }
		root.size = size
		root:render()

		bodies = getBodies(root)
	end
end

function onKey(e)
    if e.action ~= glfw.action.release then
        return
    end
end

return {
	render = render,
	update = update,
	event = event
}