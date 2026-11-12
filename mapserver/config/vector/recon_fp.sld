<?xml version="1.0" encoding="UTF-8"?>
<!--
  样式：recon_fp 图层 footprint 面。与同目录 recon_fp.xml 同名，
  map-api collect_layers() 会自动发现并作为 <layer>.sld 载入，
  驱动 WMS GetMap / WMTS 矢量栅格化出图（单规则基础样式，
  不求值 Filter/多规则分类）。删除本文件即回退到默认绿/红样式。
-->
<StyledLayerDescriptor version="1.1.0"
    xsi:schemaLocation="http://www.opengis.net/sld StyledLayerDescriptor.xsd"
    xmlns="http://www.opengis.net/sld"
    xmlns:se="http://www.opengis.net/se"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <NamedLayer>
    <se:Name>recon_fp</se:Name>
    <UserStyle>
      <se:FeatureTypeStyle>
        <se:Rule>
          <se:PolygonSymbolizer>
            <se:Fill>
              <se:SvgParameter name="fill">#2d6edc</se:SvgParameter>
              <se:SvgParameter name="fill-opacity">0.35</se:SvgParameter>
            </se:Fill>
            <se:Stroke>
              <se:SvgParameter name="stroke">#123a70</se:SvgParameter>
              <se:SvgParameter name="stroke-width">1.5</se:SvgParameter>
              <se:SvgParameter name="stroke-opacity">1</se:SvgParameter>
            </se:Stroke>
          </se:PolygonSymbolizer>
        </se:Rule>
      </se:FeatureTypeStyle>
    </UserStyle>
  </NamedLayer>
</StyledLayerDescriptor>
