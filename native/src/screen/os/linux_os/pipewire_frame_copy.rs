//! Every pointer read uses the already admitted whole Core mapping.
use super::*;
use super::super::pipewire_abi::{SpaData,SpaChunk};
use super::imports::{ImportController,ImportBacking};
use super::super::pipewire_abi::memory::fd_facts;
use crate::screen::core::capture_admission::CaptureOwner;
pub(super) fn contained(base:usize,length:usize,pointer:usize,bytes:usize)->bool {
    pointer.checked_sub(base).and_then(|offset|offset.checked_add(bytes)).is_some_and(|end|end<=length)
}
impl ImportController {
    unsafe fn owner_range(&self,block:usize,pointer:usize,bytes:usize)->io::Result<()> {
        let state=self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node=state.head.as_ref();
        while let Some(owner)=node {
            if owner.block==block && !owner.removed {
                owner.grant.check_identity(self.memory.source(),&owner.id)?;
                if owner.rights&1==0 || !contained(owner.base,owner.facts.length as usize,pointer,bytes)
                    || fd_facts(owner.fd)?!=owner.facts {
                    return Err(io::Error::other("pipewire mapped range or FD identity changed"));
                }
                return Ok(());
            }
            node=owner.next.as_ref();
        }
        Err(io::Error::other("pipewire pointer has no admitted import owner"))
    }
    pub(super) unsafe fn pointer_range(&self,pointer:*const c_void,bytes:usize)->io::Result<()> {
        let pool=self.pool.load(Ordering::Acquire);
        if pointer.is_null() || pool.is_null() {return Err(io::Error::other("pipewire null imported pointer"));}
        let block=(self.api.find_ptr)(pool,pointer);
        if block.is_null() {return Err(io::Error::other("pipewire pointer outside Core mempool"));}
        self.owner_range(block as usize,pointer as usize,bytes)
    }
    pub(super) unsafe fn data_range(&self,data:&SpaData)->io::Result<(*const u8,usize)> {
        let length=data.maxsize as usize;
        if data.type_==1 {
            self.pointer_range(data.data,length)?;return Ok((data.data.cast(),length));
        }
        if data.type_!=2 {return Err(io::Error::other("pipewire unsupported non-SHM data type"));}
        let fd=c_int::try_from(data.fd).map_err(|_|io::Error::other("pipewire invalid data FD"))?;
        let block=(self.api.find_fd)(self.pool.load(Ordering::Acquire),fd);
        if block.is_null() {return Err(io::Error::other("pipewire data FD has no Core import"));}
        let state=self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let mut node=state.head.as_ref();
        let owner:&ImportBacking=loop {
            match node {Some(value) if value.block==block as usize && !value.removed=>break value,
                Some(value)=>node=value.next.as_ref(),None=>return Err(io::Error::other("pipewire data FD is unadmitted"))}
        };
        owner.grant.check_identity(self.memory.source(),&owner.id)?;
        let offset=data.mapoffset as usize;
        let pointer=owner.base.checked_add(offset).ok_or_else(||io::Error::other("pipewire data pointer overflow"))?;
        if owner.rights&1==0 || !contained(owner.base,owner.facts.length as usize,pointer,length)
            || fd_facts(fd)?!=owner.facts {return Err(io::Error::other("pipewire data FD range changed"));}
        Ok((pointer as *const u8,length))
    }
    pub(super) unsafe fn validate_buffer(&self,buffer:*mut PwBuffer)->io::Result<()> {
        let spa=buffer.as_ref().and_then(|value|value.buffer.as_ref()).ok_or_else(||io::Error::other("pipewire null buffer"))?;
        if spa.n_datas==0 || spa.datas.is_null() {return Err(io::Error::other("pipewire no data descriptors"));}
        // Descriptors are library control objects; every variable data, chunk
        // and metadata range is validated against its actual import owner.
        for data in std::slice::from_raw_parts(spa.datas,spa.n_datas as usize) {
            self.pointer_range(data.chunk.cast(),std::mem::size_of::<SpaChunk>())?;
            if data.maxsize!=0 {self.data_range(data)?;}
        }
        if spa.n_metas!=0 {
            if spa.metas.is_null() {return Err(io::Error::other("pipewire null metadata descriptors"));}
            for meta in std::slice::from_raw_parts(spa.metas,spa.n_metas as usize) {
                if meta.size!=0 {self.pointer_range(meta.data,meta.size as usize)?;}
            }
        }Ok(())
    }
}
pub(super) fn row_span(stride:usize,size:FrameSize)->io::Result<usize> {
    let row=(size.width as usize).checked_mul(4).ok_or_else(||io::Error::other("pipewire row overflow"))?;
    if size.width==0 || size.height==0 || stride<row {return Err(io::Error::other("pipewire invalid picture stride"));}
    stride.checked_mul(size.height.saturating_sub(1) as usize).and_then(|bytes|bytes.checked_add(row))
        .ok_or_else(||io::Error::other("pipewire row span overflow"))
}
pub(super) fn chunk_span(offset:usize,chunk_bytes:usize,maxsize:usize,stride:usize,size:FrameSize)->io::Result<(usize,usize)> {
    let needed=row_span(stride,size)?;
    if offset.checked_add(chunk_bytes).map_or(true,|end|end>maxsize) || needed>chunk_bytes {
        return Err(io::Error::other("pipewire chunk smaller than its picture or outside mapped block"));
    }Ok((offset,needed))
}
pub(super) unsafe fn copy_picture(data:&CallbackData,buffer:*mut PwBuffer)->Option<CapturedImage> {
    if data.latest.import_blocked() {return None;}
    let (format,size)=(*lock_format(data))?;
    let grant=data.memory.current_grant()?;
    if grant.check_identity(data.memory.source(),data.memory.generation()).is_err()
        || !data.owners.lock().unwrap_or_else(PoisonError::into_inner).contains(buffer,grant.generation()) {return None;}
    let copy=||->io::Result<Option<CapturedImage>> {
        data.imports.validate_buffer(buffer)?;
        let block=&*(*(*buffer).buffer).datas;let chunk=&*block.chunk;
        if chunk.flags&SPA_CHUNK_FLAG_CORRUPTED!=0 || chunk.size==0 {data.latest.trace.empty_chunk();return Ok(None);}
        let stride=if chunk.stride>0 {chunk.stride as usize}else {size.width as usize*4};
        let (pointer,length)=data.imports.data_range(block)?;
        let (offset,needed)=chunk_span(chunk.offset as usize,chunk.size as usize,length,stride,size)?;
        let bytes=std::slice::from_raw_parts(pointer.add(offset),needed);
        convert_rows_admitted(bytes,stride,format,size,&grant).map(Some)
    };
    match copy() {
        Ok(image)=>image,Err(error) if error.kind()==io::ErrorKind::WouldBlock=>None,
        Err(error)=>{data.fail(error.to_string());None}
    }
}
pub(super) fn convert_rows_admitted(src:&[u8],stride:usize,format:VideoFormat,size:FrameSize,grant:&CaptureGrant)->io::Result<CapturedImage> {
    let needed=row_span(stride,size)?;
    if src.len()<needed {return Err(io::Error::other("pipewire buffer smaller than its picture"));}
    let mut image=CaptureImageBuilder::new(size,FrameLimits::RESOURCE,grant,CaptureOwner::SourcePixels)?;
    for (y,source) in src.chunks(stride).take(size.height as usize).enumerate() {
        let target=image.row_mut(y as u32).ok_or_else(||io::Error::other("pipewire row outside picture"))?;
        target.copy_from_slice(&source[..size.width as usize*4]);
        for pixel in target.chunks_exact_mut(4) {if format.swaps_red_blue() {pixel.swap(0,2);}pixel[3]=255;}
    }Ok(image.freeze())
}
