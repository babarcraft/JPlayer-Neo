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
	local w, h = table.unpack(comp.size)
	local x, y = table.unpack(comp.pos)

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
					child.render(child)
				end
			end
		end
		return props
	end,

	Rect = function(props)
		props.render = function(self)
			ui:push {
				"shapeColor",
				shape = { "rect", self.pos.x, self.pos.y, self.size.w, self.size.h },
				color = self.color
			}
		end

		return props
	end,
}


function dirty()
	dirty = true
end

function event(event)
end

function update()
end

root = Component.Root {
	children = {
		Component.Rect {
			color = { 1.0, 0.2, 0.3, 1.0 }
		}
	}
}

function render()
	if root and root.render then
		root.pos = { x = 0.0, y = 0.0 }
		root.size = ui:size()
		root.render(root)
	end
end

return {
	render = render,
	update = update,
	event = event
}