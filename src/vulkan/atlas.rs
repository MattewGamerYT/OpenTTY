use std::collections::HashMap;

use ash::vk;

use crate::font::{FontEngine, GlyphKey};

pub const ATLAS_SIZE: u32 = 2048;

#[derive(Debug, Clone, Copy)]
pub struct AtlasEntry {
    pub uv_rect: [f32; 4],
    pub width: f32,
    pub height: f32,
    pub xmin: f32,
    pub ymin: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct PendingUpload {
    pub src_offset: u64,
    pub dst_x: u32,
    pub dst_y: u32,
    pub width: u32,
    pub height: u32,
}

pub struct GlyphAtlas {
    pub image: vk::Image,
    pub memory: vk::DeviceMemory,
    pub view: vk::ImageView,
    pub sampler: vk::Sampler,

    staging_buffer: vk::Buffer,
    staging_memory: vk::DeviceMemory,
    staging_ptr: *mut u8,
    staging_capacity: u64,
    staging_cursor: u64,

    entries: HashMap<GlyphKey, AtlasEntry>,
    cursor_x: u32,
    cursor_y: u32,
    shelf_height: u32,
    needs_initial_layout: bool,
    pending_uploads: Vec<PendingUpload>,
}

impl GlyphAtlas {
    pub fn new(
        device: &ash::Device,
        mem_props: &vk::PhysicalDeviceMemoryProperties,
    ) -> Result<Self, String> {
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk::Format::R8_UNORM)
            .extent(vk::Extent3D {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_DST | vk::ImageUsageFlags::SAMPLED)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let image = unsafe { device.create_image(&image_info, None) }
            .map_err(|e| format!("create_image failed: {e}"))?;
        let reqs = unsafe { device.get_image_memory_requirements(image) };
        let mem_type = find_memory_type(
            mem_props,
            reqs.memory_type_bits,
            vk::MemoryPropertyFlags::DEVICE_LOCAL,
        )?;
        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(reqs.size)
            .memory_type_index(mem_type);
        let memory = unsafe { device.allocate_memory(&alloc_info, None) }
            .map_err(|e| format!("allocate_memory for atlas failed: {e}"))?;
        unsafe { device.bind_image_memory(image, memory, 0) }
            .map_err(|e| format!("bind_image_memory failed: {e}"))?;

        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(vk::Format::R8_UNORM)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        let view = unsafe { device.create_image_view(&view_info, None) }
            .map_err(|e| format!("create_image_view failed: {e}"))?;

        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::NEAREST)
            .min_filter(vk::Filter::NEAREST)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE);
        let sampler = unsafe { device.create_sampler(&sampler_info, None) }
            .map_err(|e| format!("create_sampler failed: {e}"))?;

        let staging_capacity: u64 = 4 * 1024 * 1024;
        let buf_info = vk::BufferCreateInfo::default()
            .size(staging_capacity)
            .usage(vk::BufferUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let staging_buffer = unsafe { device.create_buffer(&buf_info, None) }
            .map_err(|e| format!("create_buffer for atlas staging failed: {e}"))?;
        let buf_reqs = unsafe { device.get_buffer_memory_requirements(staging_buffer) };
        let staging_mem_type = find_memory_type(
            mem_props,
            buf_reqs.memory_type_bits,
            vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
        )?;
        let staging_alloc = vk::MemoryAllocateInfo::default()
            .allocation_size(buf_reqs.size)
            .memory_type_index(staging_mem_type);
        let staging_memory = unsafe { device.allocate_memory(&staging_alloc, None) }
            .map_err(|e| format!("allocate_memory for atlas staging failed: {e}"))?;
        unsafe { device.bind_buffer_memory(staging_buffer, staging_memory, 0) }
            .map_err(|e| format!("bind_buffer_memory for staging failed: {e}"))?;

        let staging_ptr = unsafe {
            device.map_memory(staging_memory, 0, staging_capacity, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| format!("map_memory for atlas staging failed: {e}"))? as *mut u8;

        Ok(Self {
            image,
            memory,
            view,
            sampler,
            staging_buffer,
            staging_memory,
            staging_ptr,
            staging_capacity,
            staging_cursor: 0,
            entries: HashMap::new(),
            cursor_x: 1,
            cursor_y: 1,
            shelf_height: 0,
            needs_initial_layout: true,
            pending_uploads: Vec::new(),
        })
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor_x = 1;
        self.cursor_y = 1;
        self.shelf_height = 0;
        self.staging_cursor = 0;
        self.pending_uploads.clear();
    }

    pub fn get_or_insert(&mut self, font: &mut FontEngine, key: GlyphKey) -> AtlasEntry {
        if let Some(&entry) = self.entries.get(&key) {
            return entry;
        }

        let raster = font.rasterize(key);
        if raster.width == 0 || raster.height == 0 {
            let empty = AtlasEntry {
                uv_rect: [0.0, 0.0, 0.0, 0.0],
                width: 0.0,
                height: 0.0,
                xmin: 0.0,
                ymin: 0.0,
            };
            self.entries.insert(key, empty);
            return empty;
        }

        let w = raster.width;
        let h = raster.height;
        let padded_w = w + 1;
        let padded_h = h + 1;

        if self.cursor_x + padded_w >= ATLAS_SIZE {
            self.cursor_x = 1;
            self.cursor_y += self.shelf_height + 1;
            self.shelf_height = 0;
        }

        if self.cursor_y + padded_h >= ATLAS_SIZE {
            // Reset atlas cleanly if saturated across scale changes.
            self.clear();
        }

        let byte_len = (w * h) as u64;
        let aligned_cursor = (self.staging_cursor + 3) & !3;
        if aligned_cursor + byte_len > self.staging_capacity {
            // Staging buffer saturated within a single frame: flush-safe reset on overflow.
            let empty = AtlasEntry {
                uv_rect: [0.0, 0.0, 0.0, 0.0],
                width: 0.0,
                height: 0.0,
                xmin: 0.0,
                ymin: 0.0,
            };
            return empty;
        }

        let dst_x = self.cursor_x;
        let dst_y = self.cursor_y;
        self.cursor_x += padded_w;
        self.shelf_height = self.shelf_height.max(padded_h);

        unsafe {
            std::ptr::copy_nonoverlapping(
                raster.bitmap.as_ptr(),
                self.staging_ptr.add(aligned_cursor as usize),
                raster.bitmap.len(),
            );
        }
        self.staging_cursor = aligned_cursor + byte_len;

        self.pending_uploads.push(PendingUpload {
            src_offset: aligned_cursor,
            dst_x,
            dst_y,
            width: w,
            height: h,
        });

        let inv = 1.0 / ATLAS_SIZE as f32;
        let entry = AtlasEntry {
            uv_rect: [
                dst_x as f32 * inv,
                dst_y as f32 * inv,
                (dst_x + w) as f32 * inv,
                (dst_y + h) as f32 * inv,
            ],
            width: w as f32,
            height: h as f32,
            xmin: raster.xmin as f32,
            ymin: raster.ymin as f32,
        };
        self.entries.insert(key, entry);
        entry
    }

    pub fn record_uploads(&mut self, device: &ash::Device, cmd: vk::CommandBuffer) {
        if !self.needs_initial_layout && self.pending_uploads.is_empty() {
            return;
        }

        let old_layout = if self.needs_initial_layout {
            vk::ImageLayout::UNDEFINED
        } else {
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL
        };

        let to_transfer = vk::ImageMemoryBarrier::default()
            .old_layout(old_layout)
            .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(if self.needs_initial_layout {
                vk::AccessFlags::empty()
            } else {
                vk::AccessFlags::SHADER_READ
            })
            .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);

        unsafe {
            device.cmd_pipeline_barrier(
                cmd,
                if self.needs_initial_layout {
                    vk::PipelineStageFlags::TOP_OF_PIPE
                } else {
                    vk::PipelineStageFlags::FRAGMENT_SHADER
                },
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_transfer],
            );
        }

        if !self.pending_uploads.is_empty() {
            let regions: Vec<vk::BufferImageCopy> = self
                .pending_uploads
                .iter()
                .map(|u| vk::BufferImageCopy {
                    buffer_offset: u.src_offset,
                    buffer_row_length: 0,
                    buffer_image_height: 0,
                    image_subresource: vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    },
                    image_offset: vk::Offset3D {
                        x: u.dst_x as i32,
                        y: u.dst_y as i32,
                        z: 0,
                    },
                    image_extent: vk::Extent3D {
                        width: u.width,
                        height: u.height,
                        depth: 1,
                    },
                })
                .collect();

            unsafe {
                device.cmd_copy_buffer_to_image(
                    cmd,
                    self.staging_buffer,
                    self.image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &regions,
                );
            }
        }

        let to_shader = vk::ImageMemoryBarrier::default()
            .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
            .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(self.image)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            })
            .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
            .dst_access_mask(vk::AccessFlags::SHADER_READ);

        unsafe {
            device.cmd_pipeline_barrier(
                cmd,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::FRAGMENT_SHADER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[to_shader],
            );
        }

        self.needs_initial_layout = false;
        self.pending_uploads.clear();
        self.staging_cursor = 0;
    }

    pub unsafe fn destroy(&mut self, device: &ash::Device) {
        unsafe {
            device.unmap_memory(self.staging_memory);
            device.destroy_buffer(self.staging_buffer, None);
            device.free_memory(self.staging_memory, None);
            device.destroy_sampler(self.sampler, None);
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

pub fn find_memory_type(
    mem_props: &vk::PhysicalDeviceMemoryProperties,
    type_filter: u32,
    required: vk::MemoryPropertyFlags,
) -> Result<u32, String> {
    for i in 0..mem_props.memory_type_count {
        let suitable = (type_filter & (1 << i)) != 0;
        let props = mem_props.memory_types[i as usize].property_flags;
        if suitable && props.contains(required) {
            return Ok(i);
        }
    }
    Err("no suitable Vulkan memory type found".to_string())
}
