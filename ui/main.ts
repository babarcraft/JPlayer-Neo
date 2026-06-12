import {ComponentBase, focused, Group, Label, Root, setRoot, Slider, TextInput, VideoSurface} from "./components";

class PlayerControls extends Root {

    player: VideoPlayerHandle | null = null
    backgroundColor: Color = [0.2, 0.2, 0.2, 0.5]
    timelineSlider: Slider;
    volumeSlider: Slider;
    passedTime: Label;
    remTime: Label;
    labelUpdateTask: TaskHandle
    timelineUpdateTask: TaskHandle
    hovered = false
    notHoveredTimes = 0
    hoveredUpdateTask: TaskHandle
    input: TextInput

    constructor() {
        let timelineSlider = new Slider("horizontal", [0.5, 0.5, 0.5, 0.2], [1, 1, 1, 1]);
        let volumeSlider = new Slider("horizontal", [0.5, 0.5, 0.5, 0.2], [1, 1, 1, 1]);
        let passedTime = new Label([1, 1, 1, 1]);
        let remTime = new Label([1, 1, 1, 1]);
        let input = new TextInput([1, 1, 1, 1], [0.3, 0.3, 0.3, 0.7])
        super([
            new Group("vertical", [
                [2, input],
                [1, new Group("horizontal", [
                    [4, null],
                    [1, volumeSlider]
                ])],
                [0.3, null],
                [1, new Group("horizontal", [
                    [1, passedTime],
                    [3, null],
                    [1, remTime],
                ])],
                [1, null],
                [1, timelineSlider],
                [0.5, null],
            ]).also(gr => {
                gr.padding = {
                    top: 5,
                    bottom: 5,
                    right: 5,
                    left: 5
                }
            }),
        ])
        this.prefSize = { h: 200 }
        this.padding = {
            top: 20,
            bottom: 20,
            right: 20,
            left: 20
        }
        this.timelineSlider = timelineSlider;
        this.passedTime = passedTime;
        this.remTime = remTime;
        this.labelUpdateTask = ui.addTask(passed => passed >= 1, () => {
            if(this.player) {
                let pts = this.player.pts;
                let duration = this.player.duration;
                passedTime.text = formatTime(pts);
                remTime.text = formatTime(duration - pts)
            }
        })
        this.timelineUpdateTask = ui.addTask(passed => passed >= 0.1, () => {
            if(this.player) {
                let pts = this.player.pts;
                let duration = this.player.duration;
                timelineSlider.progress = pts / duration
            }
        })
        timelineSlider.onNewTarget = progress => {
            if(this.player) {
                this.player.seek(this.player.duration * progress)
            }
        }
        this.hoveredUpdateTask = ui.addTask(passed => passed >= 1, () => {
            if(this.hovered) {
                this.hovered = false;
                this.notHoveredTimes = 0;
                return
            }
            if(this.visible) {
                this.notHoveredTimes += 1
                if(this.notHoveredTimes > 5) {
                    this.visible = false;
                }
            }
        })
        volumeSlider.onNewTarget = target => {
            if(this.player) {
                this.player.volume = target
                volumeSlider.progress = target
            }
        }
        volumeSlider.progress = 1
        this.input = input
    }


    render() {
        if(!this.visible) return
        let [x, y, w, h] = this.bounds();
        ui.push({
            type: "shapeFillColor",
            color: this.backgroundColor,
            shape: ["rect", x, y, w, h]
        })
        super.render()
    }

    handleEvent(event: Event): Event | null {
        switch(event.type) {
            case "mouseMoved":
                if(this.intersects(event.to)) {
                    this.hovered = true
                    this.visible = true;
                }
                break
        }
        return super.handleEvent(event);
    }
}

function formatTime(pts: number): string {
    let out = "";
    let hours = Math.floor(pts / 3600);
    if(hours < 10) out += "0"
    out += hours + ":"
    pts -= hours * 3600;
    let minutes = Math.floor(pts / 60);
    if(minutes < 10) out += "0"
    out += minutes + ":"
    pts -= minutes * 60;
    let seconds = Math.floor(pts);
    if(seconds < 10) out += "0"
    out += seconds
    return out;
}

let inputWorker = ui.newInputWorker()
let decodeWorker = ui.newDecodeWorker()

class PlayerScene extends Root {

    player?: VideoPlayerHandle = null;
    controls: PlayerControls
    surface: VideoSurface;

    constructor() {
        let surface = new VideoSurface();
        let controls = new PlayerControls();
        super([
            surface,
            controls
        ]);
        this.surface = surface;
        this.controls = controls
        controls.input.onEnterPressed = () => {
            this.player = ui.newVideoPlayer(controls.input.text.text, surface.surface, inputWorker, decodeWorker)
            this.player.play();
            controls.player = this.player
            controls.input.text.text = ""
            controls.input.text.offset = 0
        }
    }

    handleEvent(event: Event): Event | null {
        switch (event.type) {
            case "key":
                break
        }
        return super.handleEvent(event);
    }
}

setRoot(new PlayerScene())

