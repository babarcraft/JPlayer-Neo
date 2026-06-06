glfw = require("glfw")

function Root(props)
	props["type"] = "root"
	return props
end
function HGroup(props)
	props["type"] = "group"
	props["flow"] = "ho"
	return props
end

function VGroup(props)
	props["type"] = "group"
	props["flow"] = "vert"
	return props
end

function Rect(color)
	props = {
		type = "rect",
		color = color
	}
	return props
end

function VideoSurface(props)
	props["type"] = "videoSurface"
	return props
end

function Raw(props)
	props["type"] = "raw"
	return props
end

function RGB(r, g, b)
	return {
		r, g, b, 1.0
	}
end

root = Root {
	children = {
		VideoSurface {
			onDirty = function(self)
			end,
			update = function(self)
				if self.player == nil then
					self.player = createPlayer("tt.webm", self.surface)
					self.player:play()
				end
			end
		},
		VGroup {
			children = {
				{ 3.0, HGroup {
					children = {
						{ 1.0, Rect { 0.0, 1.0, 0.3, 1.0 } },
						{ 3.0, Rect { 0.1, 0.5, 0.3, 1.0 } },
					}
				} },
			},
			onDirty = function(self)
				local psize = self.parent.size;
				local size = self.size;
				self.prefSize = { size[1] - 20.0, 180.0 }
				self.prefPos = { (psize[1] - (size[1] - 20.0)) / 2.0, 10.0 }
			end,
		},
	},
	onKey = function(self)
	end,
}
