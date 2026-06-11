import {ComponentBase, Group, Label, Root, setRoot, Slider, TextInput, VideoSurface} from "./components";

class SolidRect extends ComponentBase {

    color: Color

    constructor(color: Color) {
        super();
        this.color = color
    }


    render() {
        let [x, y, w, h] = this.bounds();
        ui.push({
            type: "shapeFillColor",
            color: this.color,
            shape: ["rect", x, y, w, h]
        })
    }

}

class PlayerScene extends Root {

    player?: VideoPlayerHandle = null;
    surface: VideoSurface;
    timeline: Slider;

    constructor() {
        let surface = new VideoSurface();
        let slider = new Slider("horizontal", [0.5, 0.5, 0.5, 0.2], [1, 1, 1, 1]);
        super([
            surface,
            new Root([
                new SolidRect([0.6, 0.6, 0.2, 1]),
                new Group("vertical", [
                    [2, null],
                    [1, slider],
                    [0.3, null],
                    [1, new TextInput([1, 1, 1, 1], [1, 1, 1, 1])],
                    [2, null],
                ]),
            ]).also(gr => {
                gr.prefSize = { h: 200 }
            })
        ]);
        this.surface = surface;
        this.player = ui.newVideoPlayer("tt.webm", surface.surface)
        this.player.player.play();
        this.timeline = slider;
        ui.addTask(passed => passed > 0.5, () => {
            if(this.player) {
                let pts = this.player.player.pts;
                let duration = this.player.player.duration;
                this.timeline.progress = pts / duration;
            }
        })
    }
}

setRoot(new PlayerScene())

