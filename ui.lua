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

local function componentBounds(comp)
	local w, h = comp.size.w or 0.0, comp.size.h or 0.0
	local x, y = comp.pos.x or 0.0, comp.pos.y or 0.0

	local pref = comp.prefSize
	if pref then
		w = pref.w or w
		h = pref.h or h
	end
	local pref = comp.prefPos
	if pref then
		x = pref.x or x
		y = pref.y or y
	end

	local padding = comp.padding
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

local Component = {
	Root = function(props)
		props.render = function(self)
			for _, child in ipairs(self.children) do
				child.size = self.size;
				child.pos = self.pos;
				child.parent = self.parent;
				if child.render then
					child:render()
				end
			end
		end
		props.getChildrenList = function(self)
			local children = {}
			for i, v in pairs(self.children) do
				table.insert(children, v)
			end
			return children
		end
		props.bounds = componentBounds
		return props
	end,

	VideoPlayer = function(path)
		local surface = ui:newVideoSurface()
		local player = ui:newVideoPlayer(path, surface)
		return {
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
			bounds = componentBounds
		}
	end,

	Rect = function(props)
		props.render = function(self)
			local x, y, w, h = self:bounds(self)
			ui:push {
				"shapeColor",
				shape = { "rect", x, y, w, h },
				color = self.color
			}
		end
		props.bounds = componentBounds

		return props
	end,
}


function dirty()
	dirty = true
end

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

function update()
end

player = Component.VideoPlayer("tt.webm")
player.onMouseButton = function(self, e)
	print("Player Clicked yay!!")
	self.cmd.shape[2] = 20.0 + self.cmd.shape[2];
	self.cmd.dirty = true;
end
player.prefSize = { w = 250.0, h = 250.0 }
player.prefPos = { x = 100.0, y = 100.0 }
player:play()

root = Component.Root {
	children = {
		Component.Rect {
			color = { 1.0, 0.2, 0.3, 1.0 },
			padding = {
				bottom = 50.0,
				right = 20.0
			},
			onMouseButton = function(self, e)
				print("Rect Clicked yay!! Im... E-Rect... get it?...")
			end
		},
		player
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

return {
	render = render,
	update = update,
	event = event
}