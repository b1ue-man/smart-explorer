//! X11 target geometry is a capture/input reference, including its origin.
use super::*;
use super::super::x11_input::Area;

#[derive(Clone)]
pub(super) enum Target {Monitor {id:String},Window(Window)}

impl X11Source {
    pub(super) fn aim(&mut self,source:&SourceId)->Result<(),SourceRefusal> {
        self.target=match source {
            SourceId::Monitor(id)=>Target::Monitor {id:id.clone()},
            SourceId::Window(id)=>Target::Window(x11_windows::parse_window_id(id).ok_or_else(||refuse("window_not_found"))?),
            SourceId::Synthetic=>return Err(refuse("capture_unavailable")),
        };
        let area=self.current_area().map_err(|error|refuse(&error.to_string()))?;
        self.note_geometry(area).map_err(|_|refuse("source_exceeds_frame_limit"))
    }
    pub(super) fn current_area(&self)->io::Result<Area> {
        let area=match &self.target {
            Target::Monitor {id}=>{
                let monitors=x11_monitors::list(&self.conn,self.root)?;
                let mut matching=monitors.iter().filter(|monitor|&monitor.id==id);
                let monitor=matching.next().ok_or_else(||io::Error::new(io::ErrorKind::NotFound,"monitor_not_found"))?;
                if matching.next().is_some(){return Err(io::Error::other("ambiguous_monitor_id"));}
                Area {x:monitor.x,y:monitor.y,width:i32::try_from(monitor.width).map_err(failed)?,height:i32::try_from(monitor.height).map_err(failed)?}
            }
            Target::Window(window)=>{
                let (x,y,width,height)=x11_windows::root_geometry(&self.conn,self.root,*window)?;
                Area {x,y,width:i32::from(width),height:i32::from(height)}
            }
        };
        if area.width<=0||area.height<=0{return Err(io::Error::other("empty_capture_geometry"));}
        FrameLimits::RESOURCE.check(FrameSize::new(area.width as u32,area.height as u32))?;
        Ok(area)
    }
    pub(super) fn note_geometry(&mut self,area:Area)->io::Result<()> {
        if self.geometry!=Some(area) {
            self.reference_generation=self.reference_generation.checked_add(1)
                .ok_or_else(||io::Error::other("x11_capture_reference_exhausted"))?;
            self.geometry_changed|=self.geometry.is_some();
            self.geometry=Some(area);self.last_hash=None;self.last_image=None;
        }
        self.size=FrameSize::new(area.width as u32,area.height as u32);
        Ok(())
    }
    pub(super) fn pass(&mut self)->io::Result<Pass> {
        match self.target.clone() {
            Target::Monitor {..}=>{
                // RandR can move the output without changing root dimensions
                // or causing GetImage to fail. Refresh before every capture.
                let area=self.current_area()?;self.note_geometry(area)?;
                self.pass_monitor(area.x,area.y)
            }
            Target::Window(window)=>self.pass_window(window),
        }
    }
}
