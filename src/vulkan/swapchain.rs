use ash::vk;

pub struct SwapchainState {
    pub loader: ash::khr::swapchain::Device,
    pub swapchain: vk::SwapchainKHR,
    pub format: vk::Format,
    pub extent: vk::Extent2D,
    #[allow(dead_code)]
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    pub framebuffers: Vec<vk::Framebuffer>,
}

impl SwapchainState {
    pub fn create(
        instance: &ash::Instance,
        device: &ash::Device,
        physical_device: vk::PhysicalDevice,
        surface_loader: &ash::khr::surface::Instance,
        surface: vk::SurfaceKHR,
        render_pass: vk::RenderPass,
        desired_width: u32,
        desired_height: u32,
        old_swapchain: vk::SwapchainKHR,
    ) -> Result<Self, String> {
        let loader = ash::khr::swapchain::Device::new(instance, device);

        let caps = unsafe {
            surface_loader.get_physical_device_surface_capabilities(physical_device, surface)
        }
        .map_err(|e| format!("get_physical_device_surface_capabilities failed: {e}"))?;

        let formats = unsafe {
            surface_loader.get_physical_device_surface_formats(physical_device, surface)
        }
        .map_err(|e| format!("get_physical_device_surface_formats failed: {e}"))?;

        let present_modes = unsafe {
            surface_loader.get_physical_device_surface_present_modes(physical_device, surface)
        }
        .map_err(|e| format!("get_physical_device_surface_present_modes failed: {e}"))?;

        let surface_format = choose_surface_format(&formats);

        // Prefer MAILBOX for low-latency tear-free presentation paced by Wayland frame callbacks.
        let present_mode = if present_modes.contains(&vk::PresentModeKHR::MAILBOX) {
            vk::PresentModeKHR::MAILBOX
        } else {
            vk::PresentModeKHR::FIFO
        };

        let extent = if caps.current_extent.width != u32::MAX {
            caps.current_extent
        } else {
            vk::Extent2D {
                width: desired_width.clamp(caps.min_image_extent.width, caps.max_image_extent.width),
                height: desired_height
                    .clamp(caps.min_image_extent.height, caps.max_image_extent.height),
            }
        };

        let mut image_count = caps.min_image_count + 1;
        if caps.max_image_count > 0 && image_count > caps.max_image_count {
            image_count = caps.max_image_count;
        }

        let composite_alpha = if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::OPAQUE)
        {
            vk::CompositeAlphaFlagsKHR::OPAQUE
        } else if caps
            .supported_composite_alpha
            .contains(vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED)
        {
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED
        } else {
            vk::CompositeAlphaFlagsKHR::INHERIT
        };

        let create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(surface)
            .min_image_count(image_count)
            .image_format(surface_format.format)
            .image_color_space(surface_format.color_space)
            .image_extent(extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .image_sharing_mode(vk::SharingMode::EXCLUSIVE)
            .pre_transform(caps.current_transform)
            .composite_alpha(composite_alpha)
            .present_mode(present_mode)
            .clipped(true)
            .old_swapchain(old_swapchain);

        let swapchain = unsafe { loader.create_swapchain(&create_info, None) }
            .map_err(|e| format!("create_swapchain failed: {e}"))?;

        if old_swapchain != vk::SwapchainKHR::null() {
            unsafe { loader.destroy_swapchain(old_swapchain, None) };
        }

        let images = unsafe { loader.get_swapchain_images(swapchain) }
            .map_err(|e| format!("get_swapchain_images failed: {e}"))?;

        let mut views = Vec::with_capacity(images.len());
        let mut framebuffers = Vec::with_capacity(images.len());

        for &image in &images {
            let view_info = vk::ImageViewCreateInfo::default()
                .image(image)
                .view_type(vk::ImageViewType::TYPE_2D)
                .format(surface_format.format)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                });
            let view = unsafe { device.create_image_view(&view_info, None) }
                .map_err(|e| format!("create swapchain image view failed: {e}"))?;
            views.push(view);

            let attachments = [view];
            let fb_info = vk::FramebufferCreateInfo::default()
                .render_pass(render_pass)
                .attachments(&attachments)
                .width(extent.width)
                .height(extent.height)
                .layers(1);
            let fb = unsafe { device.create_framebuffer(&fb_info, None) }
                .map_err(|e| format!("create_framebuffer failed: {e}"))?;
            framebuffers.push(fb);
        }

        Ok(Self {
            loader,
            swapchain,
            format: surface_format.format,
            extent,
            images,
            views,
            framebuffers,
        })
    }

    pub unsafe fn destroy_views_and_framebuffers(&mut self, device: &ash::Device) {
        unsafe {
            for fb in self.framebuffers.drain(..) {
                device.destroy_framebuffer(fb, None);
            }
            for view in self.views.drain(..) {
                device.destroy_image_view(view, None);
            }
        }
    }

    pub unsafe fn destroy_all(&mut self, device: &ash::Device) {
        unsafe {
            self.destroy_views_and_framebuffers(device);
            self.loader.destroy_swapchain(self.swapchain, None);
        }
    }
}

pub fn create_render_pass(
    device: &ash::Device,
    format: vk::Format,
) -> Result<vk::RenderPass, String> {
    let attachment = vk::AttachmentDescription::default()
        .format(format)
        .samples(vk::SampleCountFlags::TYPE_1)
        .load_op(vk::AttachmentLoadOp::CLEAR)
        .store_op(vk::AttachmentStoreOp::STORE)
        .stencil_load_op(vk::AttachmentLoadOp::DONT_CARE)
        .stencil_store_op(vk::AttachmentStoreOp::DONT_CARE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .final_layout(vk::ImageLayout::PRESENT_SRC_KHR);

    let color_ref = vk::AttachmentReference::default()
        .attachment(0)
        .layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);

    let subpass = vk::SubpassDescription::default()
        .pipeline_bind_point(vk::PipelineBindPoint::GRAPHICS)
        .color_attachments(std::slice::from_ref(&color_ref));

    let dependency = vk::SubpassDependency::default()
        .src_subpass(vk::SUBPASS_EXTERNAL)
        .dst_subpass(0)
        .src_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .src_access_mask(vk::AccessFlags::empty())
        .dst_stage_mask(vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT)
        .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE);

    let rp_info = vk::RenderPassCreateInfo::default()
        .attachments(std::slice::from_ref(&attachment))
        .subpasses(std::slice::from_ref(&subpass))
        .dependencies(std::slice::from_ref(&dependency));

    unsafe { device.create_render_pass(&rp_info, None) }
        .map_err(|e| format!("create_render_pass failed: {e}"))
}

pub fn choose_surface_format(formats: &[vk::SurfaceFormatKHR]) -> vk::SurfaceFormatKHR {
    formats
        .iter()
        .copied()
        .find(|f| {
            f.format == vk::Format::B8G8R8A8_UNORM
                && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        })
        .or_else(|| {
            formats
                .iter()
                .copied()
                .find(|f| f.format == vk::Format::R8G8B8A8_UNORM)
        })
        .unwrap_or(formats[0])
}
